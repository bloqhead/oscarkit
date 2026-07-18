import Foundation
import Network

/// Wraps a raw TCP socket and speaks FLAP framing on top of it.
/// One instance = one TCP connection to either the auth server or the BOS server —
/// OSCAR logins involve connecting to both in sequence (see OSCARClient).
final class FLAPConnection {
    private let connection: NWConnection
    private var sendSequence: UInt16 = 0
    private var receiveBuffer = Data()

    /// Called for every fully-parsed frame that arrives.
    var onFrame: ((FLAPFrame) -> Void)?
    var onError: ((Error) -> Void)?
    var onReady: (() -> Void)?

    init(host: String, port: UInt16) {
        let params = NWParameters.tcp
        connection = NWConnection(
            host: NWEndpoint.Host(host),
            port: NWEndpoint.Port(rawValue: port)!,
            using: params
        )
    }

    func start() {
        connection.stateUpdateHandler = { [weak self] state in
            switch state {
            case .ready:
                self?.onReady?()
                self?.receiveLoop()
            case .failed(let error):
                self?.onError?(error)
            default:
                break
            }
        }
        connection.start(queue: .main)
    }

    func stop() {
        connection.cancel()
    }

    /// Sends a payload on the given channel, handling sequence numbering automatically.
    func send(channel: FLAPChannel, payload: Data) {
        sendSequence = sendSequence &+ 1
        let frame = FLAPFrame(channel: channel, sequence: sendSequence, payload: payload)
        connection.send(content: frame.encoded(), completion: .contentProcessed { [weak self] error in
            if let error { self?.onError?(error) }
        })
    }

    /// Convenience for sending a SNAC — wraps it as a channel-2 data frame.
    func send(snac: SNAC) {
        send(channel: .data, payload: snac.encoded())
    }

    // MARK: - Reading

    /// FLAP frames arrive as a 6-byte header followed by a variable-length payload.
    /// TCP gives us an arbitrary byte stream, so we buffer and slice out complete
    /// frames as they become available rather than assuming one read = one frame.
    private func receiveLoop() {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, isComplete, error in
            guard let self else { return }
            if let data, !data.isEmpty {
                self.receiveBuffer.append(data)
                self.drainBuffer()
            }
            if let error {
                self.onError?(error)
                return
            }
            if isComplete {
                return
            }
            self.receiveLoop()
        }
    }

    private func drainBuffer() {
        while receiveBuffer.count >= FLAPFrame.headerSize {
            let header = receiveBuffer.prefix(FLAPFrame.headerSize)
            guard let (channel, sequence, length) = FLAPFrame.parseHeader(Data(header)) else {
                // Out of sync with the stream — bail rather than spin forever.
                // In production you'd want to close the connection here.
                onError?(OSCARError.malformedFrame)
                return
            }
            let totalFrameSize = FLAPFrame.headerSize + Int(length)
            guard receiveBuffer.count >= totalFrameSize else {
                return // wait for more bytes
            }
            let payload = receiveBuffer.subdata(in: (receiveBuffer.startIndex + FLAPFrame.headerSize)..<(receiveBuffer.startIndex + totalFrameSize))
            receiveBuffer.removeSubrange(receiveBuffer.startIndex..<(receiveBuffer.startIndex + totalFrameSize))
            onFrame?(FLAPFrame(channel: channel, sequence: sequence, payload: payload))
        }
    }
}

enum OSCARError: Error {
    case malformedFrame
    case loginFailed(String)
    case unexpectedResponse
}
