import Foundation

/// FLAP is the lowest-level framing protocol OSCAR runs on top of.
/// Every single thing sent over the wire — login, IMs, buddy list updates —
/// is wrapped in a FLAP frame first.
///
/// Wire format (big-endian):
///   byte 0:      0x2a  (magic "asterisk" marker — every frame starts with this)
///   byte 1:      channel (see FLAPChannel below)
///   bytes 2-3:   sequence number (uint16, client and server each keep their own counter)
///   bytes 4-5:   payload length (uint16)
///   bytes 6...:  payload (length bytes, meaning depends on channel)
struct FLAPFrame {
    static let magicByte: UInt8 = 0x2a

    let channel: FLAPChannel
    let sequence: UInt16
    let payload: Data

    /// Serializes this frame to bytes ready to write to the socket.
    func encoded() -> Data {
        var data = Data()
        data.append(FLAPFrame.magicByte)
        data.append(channel.rawValue)
        data.append(contentsOf: sequence.bigEndianBytes)
        data.append(contentsOf: UInt16(payload.count).bigEndianBytes)
        data.append(payload)
        return data
    }

    /// The fixed 6-byte header size, useful for incremental socket reads.
    static let headerSize = 6

    /// Parses just the header to learn how many more bytes to read for the payload.
    /// Returns nil if the header doesn't start with the magic byte (out of sync / garbage).
    static func parseHeader(_ header: Data) -> (channel: FLAPChannel, sequence: UInt16, payloadLength: UInt16)? {
        guard header.count == headerSize, header[header.startIndex] == magicByte else {
            return nil
        }
        let bytes = [UInt8](header)
        guard let channel = FLAPChannel(rawValue: bytes[1]) else { return nil }
        let sequence = UInt16(bytes[2]) << 8 | UInt16(bytes[3])
        let length = UInt16(bytes[4]) << 8 | UInt16(bytes[5])
        return (channel, sequence, length)
    }
}

/// FLAP channels multiplex different kinds of traffic over the same TCP connection.
enum FLAPChannel: UInt8 {
    case newConnection = 0x01   // connection negotiation / hello / disconnect notices
    case data = 0x02            // SNAC-wrapped data — this is where almost everything lives
    case error = 0x03
    case closeConnection = 0x04
    case keepAlive = 0x05
}

extension UInt16 {
    var bigEndianBytes: [UInt8] {
        [UInt8(self >> 8), UInt8(self & 0xff)]
    }
}

extension UInt32 {
    var bigEndianBytes: [UInt8] {
        [UInt8((self >> 24) & 0xff), UInt8((self >> 16) & 0xff), UInt8((self >> 8) & 0xff), UInt8(self & 0xff)]
    }
}
