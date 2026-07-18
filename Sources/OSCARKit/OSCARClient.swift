import Foundation
import Crypto  // swift-crypto, for Insecure.MD5 — add as a package dependency (see Package.swift)

/// Orchestrates a full OSCAR login: connect to the auth server, exchange the
/// MD5-hashed password challenge, get handed off to the BOS (Basic OSCAR Service)
/// server, and land in a state where you can send/receive IMs.
///
/// This targets Open OSCAR Server's default config. Against the real (long-dead)
/// AOL servers this same flow mostly applied too — the protocol hasn't changed,
/// only who's running it.
@MainActor
final class OSCARClient: ObservableObject {

    enum State {
        case disconnected
        case connectingAuth
        case awaitingAuthKey
        case awaitingLoginResponse
        case connectingBOS
        case online
        case failed(Error)
    }

    @Published private(set) var state: State = .disconnected
    @Published private(set) var incomingMessages: [IncomingIM] = []

    /// Your synced buddy list, reconciled from feedbag + live presence updates.
    /// See Feedbag.swift for how this gets populated.
    @Published var buddies: [Buddy] = []

    /// Raw feedbag items as last synced from the server — buddies, groups, and
    /// meta-items. `buddies` above is the UI-friendly projection of this;
    /// this raw form is kept around because add/remove operations need to look
    /// up existing group IDs and item IDs.
    var feedbagItems: [FeedbagItem] = []

    struct IncomingIM: Identifiable {
        let id = UUID()
        let from: String
        let text: String
    }

    private var authConnection: FLAPConnection?
    var bosConnection: FLAPConnection?

    private let host: String
    private let authPort: UInt16
    private let screenName: String
    private let password: String

    /// - Parameters:
    ///   - host: your Open OSCAR Server's advertised hostname/IP
    ///   - authPort: default 5190, matches OSCAR_ADVERTISED_LISTENERS_PLAIN in settings.env
    init(host: String, authPort: UInt16 = 5190, screenName: String, password: String) {
        self.host = host
        self.authPort = authPort
        self.screenName = screenName
        self.password = password
    }

    // MARK: - Public API

    func login() {
        state = .connectingAuth
        let conn = FLAPConnection(host: host, port: authPort)
        authConnection = conn

        conn.onReady = { [weak self] in
            self?.beginAuthHandshake()
        }
        conn.onFrame = { [weak self] frame in
            self?.handleAuthFrame(frame)
        }
        conn.onError = { [weak self] error in
            self?.state = .failed(error)
        }
        conn.start()
    }

    func sendMessage(to recipient: String, text: String) {
        guard case .online = state, let bos = bosConnection else { return }

        // ICBM send-IM SNAC (family 0x04, subtype 0x06) body layout:
        //   8 bytes: message "cookie" (client-chosen, echoed back in acks — random is fine)
        //   2 bytes: channel (1 = plain text)
        //   TLV 0x01: recipient screen name, length-prefixed as a single byte length + chars
        //     (the *exact* byte layout of the recipient field is a "BUF" not a TLV —
        //      1 byte length + N bytes name, no type field — unlike the rest of the SNAC)
        //   TLV 0x02: message data, itself containing nested fragments (0x0501 = features, 0x0101 = text)
        var body = Data()
        body.append(contentsOf: (0..<8).map { _ in UInt8.random(in: 0...255) }) // cookie
        body.append(contentsOf: UInt16(1).bigEndianBytes) // channel 1

        let nameBytes = Array(recipient.utf8)
        body.append(UInt8(nameBytes.count))
        body.append(contentsOf: nameBytes)

        // Message TLV (type 0x02) wraps two inner fragments.
        var messageInner = Data()
        // Feature fragment — clients usually send a fixed "capabilities" blob here;
        // an empty/minimal one is tolerated by most permissive OSCAR servers.
        let featureFragment = TLV(type: 0x0501, value: Data([0x01, 0x01, 0x01, 0x02]))
        messageInner.append(featureFragment.encoded())
        let textFragment = TLV(type: 0x0101, value: Data([0x00, 0x00]) + Data(text.utf8)) // charset + charsubset + text
        messageInner.append(textFragment.encoded())

        let messageTLV = TLV(type: 0x02, value: messageInner)
        body.append(messageTLV.encoded())

        let header = SNACHeader(family: SNACFamily.messaging.rawValue, subtype: 0x06, flags: 0, requestID: nextRequestID())
        bos.send(snac: SNAC(header: header, body: body))
    }

    // MARK: - Auth server handshake

    private func beginAuthHandshake() {
        guard let conn = authConnection else { return }
        // Channel 1 "hello": 4-byte FLAP protocol version, always 1.
        conn.send(channel: .newConnection, payload: Data(UInt32(1).bigEndianBytes))

        // Request an auth key by sending our screen name.
        // SNAC family 0x17 (BUCP), subtype 0x06 = "request login challenge".
        state = .awaitingAuthKey
        let nameTLV = TLV(type: 0x01, value: Data(screenName.utf8))
        let header = SNACHeader(family: SNACFamily.authorization.rawValue, subtype: 0x06, flags: 0, requestID: nextRequestID())
        conn.send(snac: SNAC(header: header, body: nameTLV.encoded()))
    }

    private func handleAuthFrame(_ frame: FLAPFrame) {
        guard frame.channel == .data, let snac = SNAC.parse(frame.payload) else { return }

        switch (snac.header.family, snac.header.subtype) {
        case (SNACFamily.authorization.rawValue, 0x07):
            // Server sent us the auth key (challenge). TLV 0x01 contains it.
            let tlvs = TLV.parseAll(snac.body)
            guard let authKey = tlvs[0x01] else {
                state = .failed(OSCARError.unexpectedResponse)
                return
            }
            sendLoginRequest(authKey: authKey)

        case (SNACFamily.authorization.rawValue, 0x03):
            // Login response: either an error (TLV 0x08) or success with BOS address + cookie.
            let tlvs = TLV.parseAll(snac.body)
            if let errorData = tlvs[0x08] {
                let code = errorData.withUnsafeBytes { $0.load(as: UInt16.self) }.bigEndian
                state = .failed(OSCARError.loginFailed("BUCP error code \(code)"))
                return
            }
            guard let bosAddressData = tlvs[0x05], let cookie = tlvs[0x06],
                  let bosAddress = String(data: bosAddressData, encoding: .ascii) else {
                state = .failed(OSCARError.unexpectedResponse)
                return
            }
            authConnection?.stop()
            connectToBOS(address: bosAddress, cookie: cookie)

        default:
            break
        }
    }

    private func sendLoginRequest(authKey: Data) {
        state = .awaitingLoginResponse
        // Roasted MD5: MD5( authKey + MD5(password) + "AOL Instant Messenger (SM)" )
        // This specific chained-hash scheme is what libpurple's OSCAR module uses
        // and is well documented in that codebase if you need to cross-check it.
        let passwordDigest = Insecure.MD5.hash(data: Data(password.utf8))
        var combined = Data()
        combined.append(authKey)
        combined.append(contentsOf: passwordDigest)
        combined.append(Data("AOL Instant Messenger (SM)".utf8))
        let finalHash = Insecure.MD5.hash(data: combined)

        var body = Data()
        body.append(TLV(type: 0x01, value: Data(screenName.utf8)).encoded())
        body.append(TLV(type: 0x25, value: Data(finalHash)).encoded())
        body.append(TLV(type: 0x03, value: Data("OSCARKit/0.1".utf8)).encoded()) // client ID string

        let header = SNACHeader(family: SNACFamily.authorization.rawValue, subtype: 0x02, flags: 0, requestID: nextRequestID())
        authConnection?.send(snac: SNAC(header: header, body: body))
    }

    // MARK: - BOS connection

    private func connectToBOS(address: String, cookie: Data) {
        state = .connectingBOS
        let parts = address.split(separator: ":")
        let bosHost = String(parts.first ?? Substring(host))
        let bosPort = UInt16(parts.count > 1 ? String(parts[1]) : nil) ?? authPort

        let conn = FLAPConnection(host: bosHost, port: bosPort)
        bosConnection = conn

        conn.onReady = { [weak self] in
            guard let self else { return }
            // Channel 1 hello again, but this time carrying the auth cookie as a TLV
            // so the BOS server knows which just-authenticated session this is.
            var payload = Data(UInt32(1).bigEndianBytes)
            payload.append(TLV(type: 0x06, value: cookie).encoded())
            conn.send(channel: .newConnection, payload: payload)
        }
        conn.onFrame = { [weak self] frame in
            self?.handleBOSFrame(frame)
        }
        conn.onError = { [weak self] error in
            self?.state = .failed(error)
        }
        conn.start()
    }

    private func handleBOSFrame(_ frame: FLAPFrame) {
        guard frame.channel == .data, let snac = SNAC.parse(frame.payload) else { return }

        switch (snac.header.family, snac.header.subtype) {
        case (SNACFamily.generic.rawValue, 0x03):
            // "Host online" — server telling us which SNAC families it supports.
            // Real clients negotiate rate limits (family 0x01, subtype 0x06/0x07) here
            // before doing anything else; skipping that is fine against a permissive
            // self-hosted server but add it if you hit rate-limit disconnects.
            state = .online
            // Roster is foundational session state — fetch it as soon as we're online,
            // same as real clients do before anything else becomes meaningful.
            requestBuddyList()

        case (SNACFamily.messaging.rawValue, 0x07):
            // Incoming IM (ICBM "channel message"). Parsing the nested TLV/fragment
            // structure fully is more involved than the send path — this is a
            // best-effort extraction of the sender name and plain text body.
            if let im = Self.parseIncomingIM(snac.body) {
                incomingMessages.append(im)
            }

        case (SNACFamily.feedbag.rawValue, _):
            handleFeedbagFrame(snac)

        case (SNACFamily.buddyPresence.rawValue, _):
            handlePresenceFrame(snac)

        default:
            break
        }
    }

    private static func parseIncomingIM(_ body: Data) -> IncomingIM? {
        // Layout: 8-byte cookie, 2-byte channel, then a BUF (1-byte length + name),
        // then TLVs including 0x02 (message data) containing nested fragments.
        guard body.count > 11 else { return nil }
        var index = body.startIndex + 10 // skip cookie + channel
        let nameLength = Int(body[index])
        index += 1
        guard index + nameLength <= body.endIndex else { return nil }
        let sender = String(data: body[index..<index + nameLength], encoding: .utf8) ?? "unknown"
        index += nameLength

        let rest = body[index...]
        let tlvs = TLV.parseAll(Data(rest))
        guard let messageTLV = tlvs[0x02] else { return IncomingIM(from: sender, text: "") }

        // Inside the message TLV: nested fragments, each itself type/length/value.
        let fragments = TLV.parseAll(messageTLV)
        guard let textFragment = fragments[0x0101], textFragment.count > 2 else {
            return IncomingIM(from: sender, text: "")
        }
        let text = String(data: textFragment.dropFirst(2), encoding: .utf8) ?? ""
        return IncomingIM(from: sender, text: text)
    }

    // MARK: - Helpers

    private var requestIDCounter: UInt32 = 0
    func nextRequestID() -> UInt32 {
        requestIDCounter &+= 1
        return requestIDCounter
    }

    // Feedbag item IDs are scoped per-account, chosen by the client, and must
    // not collide with existing items. A monotonic counter seeded above any
    // ID we've seen from the server is good enough for a v0.1 — a real app
    // should persist the high-water mark rather than restart from 1 each launch.
    private var feedbagItemIDCounter: UInt16 = 1
    func nextFeedbagItemID() -> UInt16 {
        let existingMax = feedbagItems.map(\.itemID).max() ?? 0
        feedbagItemIDCounter = max(feedbagItemIDCounter, existingMax) &+ 1
        return feedbagItemIDCounter
    }
}
