import Foundation

/// "Feedbag" is OSCAR's internal name for the buddy list service (SNAC family 0x13).
/// Nobody seems to know why it's called that — it predates any documentation that
/// explains it — but the name stuck in every implementation including this one,
/// mostly so you can grep protocol docs and libpurple source for matching terms.
///
/// The key idea: your buddy list isn't a client-local thing. It's server state,
/// synced down on login and mutated via insert/update/delete requests. A client
/// that "adds a buddy" locally without telling the server isn't really adding
/// a buddy in any OSCAR-meaningful sense — it'll vanish next login.
enum FeedbagSubtype {
    static let rightsQuery: UInt16 = 0x02   // client: "what are the limits?"
    static let rightsReply: UInt16 = 0x03
    static let query: UInt16 = 0x04         // client: "send me my whole list"
    static let reply: UInt16 = 0x05         // server: here's your list
    static let use: UInt16 = 0x06           // client: "ack, I've got it, proceed"
    static let insertItem: UInt16 = 0x08    // client: add buddy/group
    static let updateItem: UInt16 = 0x09
    static let deleteItem: UInt16 = 0x0A
    static let status: UInt16 = 0x0E        // server: ack of insert/update/delete
}

/// Every entry in a feedbag — a buddy, a group, or a handful of special
/// metadata items (permit/deny lists, visibility prefs) — shares this same
/// wire structure. `classID` is what tells you which kind you're looking at.
struct FeedbagItem {
    let name: String
    let groupID: UInt16
    let itemID: UInt16
    let classID: UInt16
    let attributes: Data // raw TLV block; parse with TLV.parseAll if you need specific fields

    // Known classIDs. There are more (icon metadata, ignore list, etc.) —
    // add as needed.
    static let classBuddy: UInt16 = 0x0000
    static let classGroup: UInt16 = 0x0001
    static let classPermit: UInt16 = 0x0002
    static let classDeny: UInt16 = 0x0003
    static let classPermitDenyPrefs: UInt16 = 0x0004
    static let classRootGroup: UInt16 = 0x0000 // groupID 0 + classGroup = the implicit top-level group

    func encoded() -> Data {
        var data = Data()
        let nameBytes = Array(name.utf8)
        data.append(contentsOf: UInt16(nameBytes.count).bigEndianBytes)
        data.append(contentsOf: nameBytes)
        data.append(contentsOf: groupID.bigEndianBytes)
        data.append(contentsOf: itemID.bigEndianBytes)
        data.append(contentsOf: classID.bigEndianBytes)
        data.append(contentsOf: UInt16(attributes.count).bigEndianBytes)
        data.append(attributes)
        return data
    }

    /// Parses one item starting at `data`'s startIndex, returning the item and
    /// how many bytes it consumed so the caller can advance through a run of them.
    static func parse(_ data: Data) -> (item: FeedbagItem, consumed: Int)? {
        var index = data.startIndex
        func readUInt16() -> UInt16? {
            guard index + 2 <= data.endIndex else { return nil }
            let value = UInt16(data[index]) << 8 | UInt16(data[index + 1])
            index += 2
            return value
        }
        guard let nameLength = readUInt16() else { return nil }
        guard index + Int(nameLength) <= data.endIndex else { return nil }
        let name = String(data: data[index..<index + Int(nameLength)], encoding: .utf8) ?? ""
        index += Int(nameLength)

        guard let groupID = readUInt16(), let itemID = readUInt16(), let classID = readUInt16() else { return nil }
        guard let attrLength = readUInt16() else { return nil }
        guard index + Int(attrLength) <= data.endIndex else { return nil }
        let attributes = Data(data[index..<index + Int(attrLength)])
        index += Int(attrLength)

        let item = FeedbagItem(name: name, groupID: groupID, itemID: itemID, classID: classID, attributes: attributes)
        return (item, index - data.startIndex)
    }

    /// Parses a run of consecutive items, consuming as many as fit.
    static func parseAll(_ data: Data) -> [FeedbagItem] {
        var items: [FeedbagItem] = []
        var remaining = data
        while !remaining.isEmpty {
            guard let (item, consumed) = parse(remaining), consumed > 0 else { break }
            items.append(item)
            remaining = remaining.dropFirst(consumed)
        }
        return items
    }
}

/// A buddy resolved from feedbag + live presence, ready for UI consumption.
/// `isOnline` gets flipped by family 0x03 (Buddy) arrival/departure notifications,
/// which arrive as a separate stream from the feedbag list itself.
struct Buddy: Identifiable, Hashable {
    var id: String { screenName }
    let screenName: String
    let groupName: String
    var isOnline: Bool = false
}

extension OSCARClient {

    /// Kick off the buddy-list fetch. Call this once you're `.online` —
    /// typically right after "host online" arrives, before you do anything else,
    /// since real clients treat the roster as foundational session state.
    func requestBuddyList() {
        guard case .online = state, let bos = bosConnection else { return }
        let header = SNACHeader(family: SNACFamily.feedbag.rawValue, subtype: FeedbagSubtype.query, flags: 0, requestID: nextRequestID())
        bos.send(snac: SNAC(header: header, body: Data()))
    }

    /// Adds a buddy to a named group, creating the group locally if it doesn't
    /// exist yet in your tracked item-ID space. Server is the source of truth —
    /// this optimistically updates local state and relies on the 0x0E "status"
    /// ack to confirm; a real app should reconcile on mismatch.
    func addBuddy(screenName: String, toGroup groupName: String) {
        guard case .online = state, let bos = bosConnection else { return }

        let groupID = groupID(for: groupName)
        let itemID = nextFeedbagItemID()

        let item = FeedbagItem(name: screenName, groupID: groupID, itemID: itemID, classID: FeedbagItem.classBuddy, attributes: Data())
        let header = SNACHeader(family: SNACFamily.feedbag.rawValue, subtype: FeedbagSubtype.insertItem, flags: 0, requestID: nextRequestID())
        bos.send(snac: SNAC(header: header, body: item.encoded()))

        // Optimistic local update — reconciled for real once 0x0E status comes back.
        buddies.append(Buddy(screenName: screenName, groupName: groupName))
    }

    func removeBuddy(screenName: String) {
        guard case .online = state, let bos = bosConnection,
              let existing = feedbagItems.first(where: { $0.classID == FeedbagItem.classBuddy && $0.name == screenName }) else { return }

        let header = SNACHeader(family: SNACFamily.feedbag.rawValue, subtype: FeedbagSubtype.deleteItem, flags: 0, requestID: nextRequestID())
        bos.send(snac: SNAC(header: header, body: existing.encoded()))
        buddies.removeAll { $0.screenName == screenName }
    }

    // MARK: - Frame handling (called from handleBOSFrame's dispatch)

    func handleFeedbagFrame(_ snac: SNAC) {
        switch snac.header.subtype {
        case FeedbagSubtype.reply:
            handleFeedbagReply(snac.body)
        case FeedbagSubtype.status:
            // Per-item ack of a prior insert/update/delete. Body is a run of
            // uint16 result codes, one per item in the original request —
            // fine to ignore for a v0.1 given we're already updating optimistically.
            break
        default:
            break
        }
    }

    /// Family 0x03 (Buddy) — presence notifications, separate from the roster itself.
    func handlePresenceFrame(_ snac: SNAC) {
        switch snac.header.subtype {
        case 0x0B: // buddy arrived (online)
            if let name = Self.parseScreenNameBUF(snac.body) {
                setOnline(true, for: name)
            }
        case 0x0C: // buddy departed (offline)
            if let name = Self.parseScreenNameBUF(snac.body) {
                setOnline(false, for: name)
            }
        default:
            break
        }
    }

    private func handleFeedbagReply(_ body: Data) {
        // Layout (best-effort — verify against a Wireshark capture of Pidgin
        // logging into your server, same caveat as the rest of this scaffold):
        //   1 byte:  version
        //   2 bytes: item count
        //   N items, back to back (FeedbagItem.parseAll handles this part)
        //   4 bytes: last-modification timestamp (trailing, can be ignored on first sync)
        guard body.count > 3 else { return }
        let itemCount = Int(UInt16(body[body.startIndex + 1]) << 8 | UInt16(body[body.startIndex + 2]))
        let itemsData = body.dropFirst(3)
        let items = FeedbagItem.parseAll(itemsData)

        feedbagItems = items

        // Build group-ID -> name lookup first, since buddy items only carry a groupID.
        var groupNames: [UInt16: String] = [0: "Buddies"] // root/ungrouped fallback
        for item in items where item.classID == FeedbagItem.classGroup {
            groupNames[item.itemID] = item.name
        }

        buddies = items
            .filter { $0.classID == FeedbagItem.classBuddy }
            .map { Buddy(screenName: $0.name, groupName: groupNames[$0.groupID] ?? "Buddies") }
            .prefix(itemCount) // sanity bound, in case parsing overshoots
            .map { $0 }

        // Ack receipt so the server proceeds — some implementations wait for
        // this before sending anything further.
        if let bos = bosConnection {
            let header = SNACHeader(family: SNACFamily.feedbag.rawValue, subtype: FeedbagSubtype.use, flags: 0, requestID: nextRequestID())
            bos.send(snac: SNAC(header: header, body: Data()))
        }
    }

    private func setOnline(_ online: Bool, for screenName: String) {
        guard let index = buddies.firstIndex(where: { $0.screenName == screenName }) else { return }
        buddies[index].isOnline = online
    }

    private static func parseScreenNameBUF(_ body: Data) -> String? {
        // Family 0x03 arrival/departure bodies lead with the same BUF pattern
        // as ICBM: 1-byte length + name bytes.
        guard let first = body.first else { return nil }
        let length = Int(first)
        guard body.count >= 1 + length else { return nil }
        return String(data: body.dropFirst().prefix(length), encoding: .utf8)
    }

    private func groupID(for name: String) -> UInt16 {
        if let existing = feedbagItems.first(where: { $0.classID == FeedbagItem.classGroup && $0.name == name }) {
            return existing.itemID
        }
        // New group: create it too. Real clients send both the group item and
        // the buddy item in one insertItem SNAC with multiple items concatenated;
        // simplified here to two separate calls for clarity.
        let newGroupID = nextFeedbagItemID()
        if let bos = bosConnection {
            let groupItem = FeedbagItem(name: name, groupID: 0, itemID: newGroupID, classID: FeedbagItem.classGroup, attributes: Data())
            let header = SNACHeader(family: SNACFamily.feedbag.rawValue, subtype: FeedbagSubtype.insertItem, flags: 0, requestID: nextRequestID())
            bos.send(snac: SNAC(header: header, body: groupItem.encoded()))
            feedbagItems.append(groupItem)
        }
        return newGroupID
    }
}
