import Foundation

/// A SNAC ("Simple Network Atomic Communication") is the actual command unit
/// inside a FLAP channel-2 data frame. Every login step, IM, buddy list update,
/// etc. is a SNAC identified by a (family, subtype) pair — e.g. family 0x04
/// is "ICBM" (messaging), subtype 0x06 is "send IM".
///
/// Wire format (big-endian), all inside the FLAP payload:
///   bytes 0-1: family   (uint16)
///   bytes 2-3: subtype  (uint16)
///   bytes 4-5: flags    (uint16, usually 0)
///   bytes 6-9: requestID (uint32, client picks this, server echoes it back — useful for matching responses)
///   bytes 10...: data (family/subtype specific, see below)
struct SNACHeader {
    let family: UInt16
    let subtype: UInt16
    let flags: UInt16
    let requestID: UInt32

    static let size = 10

    func encoded() -> Data {
        var data = Data()
        data.append(contentsOf: family.bigEndianBytes)
        data.append(contentsOf: subtype.bigEndianBytes)
        data.append(contentsOf: flags.bigEndianBytes)
        data.append(contentsOf: requestID.bigEndianBytes)
        return data
    }

    static func parse(_ data: Data) -> SNACHeader? {
        guard data.count >= size else { return nil }
        let bytes = [UInt8](data.prefix(size))
        let family = UInt16(bytes[0]) << 8 | UInt16(bytes[1])
        let subtype = UInt16(bytes[2]) << 8 | UInt16(bytes[3])
        let flags = UInt16(bytes[4]) << 8 | UInt16(bytes[5])
        let requestID = UInt32(bytes[6]) << 24 | UInt32(bytes[7]) << 16 | UInt32(bytes[8]) << 8 | UInt32(bytes[9])
        return SNACHeader(family: family, subtype: subtype, flags: flags, requestID: requestID)
    }
}

/// The SNAC families we actually implement in this scaffold.
/// There are many more (buddy lists, chat rooms, file transfer...) —
/// add them here as you build those features out.
enum SNACFamily: UInt16 {
    case generic = 0x0001        // service-level: rate limits, host online/offline
    case locate = 0x0002         // user profile + away message get/set
    case buddyPresence = 0x0003  // "Buddy" family — online/offline arrival notifications
    case messaging = 0x0004      // ICBM — instant messages
    case feedbag = 0x0013        // buddy list roster storage (add/remove/sync)
    case authorization = 0x0017  // BUCP — login/auth
}

/// A parsed SNAC: header + raw body. Callers decode the body based on family/subtype.
struct SNAC {
    let header: SNACHeader
    let body: Data

    func encoded() -> Data {
        header.encoded() + body
    }

    static func parse(_ data: Data) -> SNAC? {
        guard let header = SNACHeader.parse(data) else { return nil }
        let body = data.suffix(from: data.startIndex + SNACHeader.size)
        return SNAC(header: header, body: Data(body))
    }
}

// MARK: - TLV (Type-Length-Value) encoding

/// Most SNAC payloads are built from TLVs rather than fixed structs —
/// e.g. the login request is a bag of TLVs (screen name, password hash, client version...).
/// Wire format: type (uint16), length (uint16), value (length bytes).
struct TLV {
    let type: UInt16
    let value: Data

    func encoded() -> Data {
        var data = Data()
        data.append(contentsOf: type.bigEndianBytes)
        data.append(contentsOf: UInt16(value.count).bigEndianBytes)
        data.append(value)
        return data
    }

    /// Parses a flat run of consecutive TLVs (this is how most SNAC bodies are structured).
    static func parseAll(_ data: Data) -> [UInt16: Data] {
        var result: [UInt16: Data] = [:]
        var index = data.startIndex
        while index + 4 <= data.endIndex {
            let bytes = [UInt8](data[index..<index + 4])
            let type = UInt16(bytes[0]) << 8 | UInt16(bytes[1])
            let length = Int(UInt16(bytes[2]) << 8 | UInt16(bytes[3]))
            let valueStart = index + 4
            guard valueStart + length <= data.endIndex else { break }
            result[type] = Data(data[valueStart..<valueStart + length])
            index = valueStart + length
        }
        return result
    }
}
