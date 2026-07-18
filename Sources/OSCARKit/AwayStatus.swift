import Foundation

/// The Locate family (SNAC 0x02) is OSCAR's mechanism for both user profiles
/// and away messages — they're the same underlying concept ("info about a
/// user that gets fetched on demand"), just different TLV slots in the same
/// SET_INFO / USER_INFO_REPLY structures.
///
/// The quirk worth internalizing: there's no dedicated "go away" or "come
/// back" command. Setting your away message *is* going away. Sending a
/// SET_INFO with an empty away TLV *is* coming back. The presence system
/// (family 0x03, already wired up in Feedbag.swift) picks up the resulting
/// status-bit change and broadcasts it to your buddies automatically — you
/// don't separately announce "I'm away" beyond setting the message itself.
enum LocateSubtype {
    static let setInfo: UInt16 = 0x04          // client: set my profile/away message
    static let userInfoQuery: UInt16 = 0x05    // client: "tell me about this buddy"
    static let userInfoReply: UInt16 = 0x06    // server: here's their info
}

// TLV types used inside both SET_INFO (outgoing) and USER_INFO_REPLY (incoming).
private enum LocateTLV {
    static let profileEncoding: UInt16 = 0x01
    static let profileText: UInt16 = 0x02
    static let awayEncoding: UInt16 = 0x03
    static let awayText: UInt16 = 0x04
}

extension OSCARClient {

    /// Sets (or clears, if `nil`) your away message. This is the *only* away
    /// mechanism in OSCAR — there's no separate "toggle away mode" — sending
    /// non-empty text here is what makes you appear away to buddies; sending
    /// `nil` sends an empty TLV, which is how you come back.
    func setAwayMessage(_ text: String?) {
        guard case .online = state, let bos = bosConnection else { return }

        var body = Data()
        // Encoding TLVs use a fixed charset string, same convention as message
        // fragments elsewhere in the protocol.
        body.append(TLV(type: LocateTLV.awayEncoding, value: Data("us-ascii".utf8)).encoded())
        body.append(TLV(type: LocateTLV.awayText, value: Data((text ?? "").utf8)).encoded())

        let header = SNACHeader(family: SNACFamily.locate.rawValue, subtype: LocateSubtype.setInfo, flags: 0, requestID: nextRequestID())
        bos.send(snac: SNAC(header: header, body: body))

        // Optimistic local update — this is *your own* state, so there's no
        // server round-trip needed to know it took effect the way there is
        // for e.g. buddy list inserts.
        awayMessage = text
    }

    /// Requests a buddy's current profile/away message. Reply arrives async
    /// via `handleLocateFrame` and updates the matching entry in `buddies`.
    func requestUserInfo(for screenName: String) {
        guard case .online = state, let bos = bosConnection else { return }

        var body = Data()
        body.append(TLV(type: 0x01, value: Data(screenName.utf8)).encoded())
        // Request flags bitmask — 0x0001 asks for away message specifically.
        // (Profile text, capabilities, etc. have their own bits; add as needed.)
        body.append(TLV(type: 0x02, value: Data(UInt16(0x0001).bigEndianBytes)).encoded())

        let header = SNACHeader(family: SNACFamily.locate.rawValue, subtype: LocateSubtype.userInfoQuery, flags: 0, requestID: nextRequestID())
        bos.send(snac: SNAC(header: header, body: body))
    }

    /// Called from handleBOSFrame's dispatch for family 0x02 traffic.
    func handleLocateFrame(_ snac: SNAC) {
        guard snac.header.subtype == LocateSubtype.userInfoReply else { return }

        // Layout: BUF screen name (1-byte length + chars), then a TLV block
        // with the same profile/away TLVs used in SET_INFO.
        guard let first = snac.body.first else { return }
        let nameLength = Int(first)
        guard snac.body.count >= 1 + nameLength else { return }
        let screenName = String(data: snac.body.dropFirst().prefix(nameLength), encoding: .utf8) ?? ""

        let rest = snac.body.dropFirst(1 + nameLength)
        let tlvs = TLV.parseAll(Data(rest))
        let awayText = tlvs[LocateTLV.awayText].flatMap { String(data: $0, encoding: .utf8) }

        if let index = buddies.firstIndex(where: { $0.screenName == screenName }) {
            buddies[index].awayMessage = (awayText?.isEmpty == false) ? awayText : nil
        }
    }
}
