//! "Feedbag" is OSCAR's internal name for the buddy list service (SNAC
//! family 0x13). Nobody seems to know why it's called that — it predates
//! any documentation that explains it — but the name stuck in every
//! implementation including this one, mostly so you can grep protocol docs
//! and libpurple source for matching terms.
//!
//! The key idea: your buddy list isn't a client-local thing. It's server
//! state, synced down on login and mutated via insert/update/delete
//! requests. A client that "adds a buddy" locally without telling the
//! server isn't really adding a buddy in any OSCAR-meaningful sense — it'll
//! vanish next login.

use std::collections::HashMap;

use crate::client::{screen_names_match, OscarError, OscarSession};
use crate::snac::{Snac, SnacFamily, SnacHeader, Tlv, UserInfo};

// Subtype numbers confirmed against Open OSCAR Server's wire.Feedbag*
// constants — REPLY and USE were both off by one (0x05/0x06 instead of the
// real 0x06/0x07). 0x05 is actually a *different* client-to-server message
// (FeedbagQueryIfModified) this codebase never sends, so the old REPLY
// match arm silently never fired against a real server: every real feedbag
// reply fell through to the default no-op case, and the buddy list never
// synced from a real server at all.
const QUERY: u16 = 0x04; // client: "send me my whole list"
const REPLY: u16 = 0x06; // server: here's your list
const USE: u16 = 0x07; // client: "ack, I've got it, proceed"
const INSERT_ITEM: u16 = 0x08; // client: add buddy/group
const DELETE_ITEM: u16 = 0x0A;
const STATUS: u16 = 0x0E; // server: ack of insert/update/delete

/// Every entry in a feedbag — a buddy, a group, or a handful of special
/// metadata items (permit/deny lists, visibility prefs) — shares this same
/// wire structure. `class_id` is what tells you which kind you're looking
/// at.
///
/// Confirmed against Open OSCAR Server's `wire.FeedbagItem`:
/// Name length prefix is a plain **2-byte** (`u16`) length — confirmed
/// empirically from a real feedbag reply's raw bytes (`oscar:"len_prefix=uint8"`
/// was an earlier, wrong reading of Open OSCAR Server's source; a second
/// item's name only decoded correctly, matching actual buddy-list content,
/// once read as `u16`, so this went with the byte-level evidence over the
/// secondhand source summary). `attributes` is a `TLVBlock` (a TLV *count*
/// prefix + that many TLVs), not a raw byte-length-prefixed blob.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbagItem {
    pub name: String,
    pub group_id: u16,
    pub item_id: u16,
    pub class_id: u16,
    pub attributes: Vec<Tlv>,
}

impl FeedbagItem {
    // Known class IDs. There are more (icon metadata, ignore list, etc.) —
    // add as needed. CLASS_PERMIT (0x0002, the allow-list counterpart to
    // CLASS_DENY) stays out for now since nothing uses it yet.
    pub const CLASS_BUDDY: u16 = 0x0000;
    pub const CLASS_GROUP: u16 = 0x0001;
    pub const CLASS_DENY: u16 = 0x0003; // block list — see add_to_block_list

    pub fn encode(&self) -> Vec<u8> {
        let name_bytes = self.name.as_bytes();
        let mut out = Vec::with_capacity(10 + name_bytes.len());
        out.extend_from_slice(&(name_bytes.len() as u16).to_be_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(&self.group_id.to_be_bytes());
        out.extend_from_slice(&self.item_id.to_be_bytes());
        out.extend_from_slice(&self.class_id.to_be_bytes());
        out.extend_from_slice(&(self.attributes.len() as u16).to_be_bytes());
        for tlv in &self.attributes {
            out.extend(tlv.encode());
        }
        out
    }

    /// Parses one item starting at the front of `data`, returning the item
    /// and how many bytes it consumed so the caller can advance through a
    /// run of them.
    pub fn parse(data: &[u8]) -> Option<(FeedbagItem, usize)> {
        fn read_u16(data: &[u8], index: &mut usize) -> Option<u16> {
            if *index + 2 > data.len() {
                return None;
            }
            let value = u16::from_be_bytes([data[*index], data[*index + 1]]);
            *index += 2;
            Some(value)
        }

        let mut index = 0usize;
        let name_len = read_u16(data, &mut index)? as usize;
        if index + name_len > data.len() {
            return None;
        }
        let name = String::from_utf8_lossy(&data[index..index + name_len]).to_string();
        index += name_len;

        let group_id = read_u16(data, &mut index)?;
        let item_id = read_u16(data, &mut index)?;
        let class_id = read_u16(data, &mut index)?;
        let attr_count = read_u16(data, &mut index)? as usize;
        let (attr_tlvs, consumed) = Tlv::parse_n(&data[index..], attr_count);
        let attributes: Vec<Tlv> = attr_tlvs.into_iter().map(|(tlv_type, value)| Tlv { tlv_type, value }).collect();
        index += consumed;

        Some((FeedbagItem { name, group_id, item_id, class_id, attributes }, index))
    }

    /// Parses a run of consecutive items, consuming as many as fit.
    pub fn parse_all(data: &[u8]) -> Vec<FeedbagItem> {
        let mut items = Vec::new();
        let mut remaining = data;
        while !remaining.is_empty() {
            let Some((item, consumed)) = FeedbagItem::parse(remaining) else { break };
            if consumed == 0 {
                break;
            }
            items.push(item);
            remaining = &remaining[consumed..];
        }
        items
    }
}

/// A buddy resolved from feedbag + live presence, ready for UI consumption.
/// `is_online` gets flipped by family 0x03 (Buddy) arrival/departure
/// notifications, which arrive as a separate stream from the feedbag list
/// itself. `is_away`/`away_message` are populated from presence flags and
/// the Locate family respectively — see `locate.rs`. `warning_level` comes
/// from presence arrivals and ICBM warning replies (see `messaging.rs`);
/// `is_blocked` reflects CLASS_DENY membership in the synced feedbag.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Buddy {
    pub screen_name: String,
    pub group_name: String,
    pub is_online: bool,
    pub is_away: bool,
    pub away_message: Option<String>,
    pub warning_level: u16,
    pub is_blocked: bool,
}

impl OscarSession {
    /// Kick off the buddy-list fetch. `login()` already calls this once BOS
    /// comes online, since real clients treat the roster as foundational
    /// session state.
    pub async fn request_buddy_list(&mut self) -> Result<(), OscarError> {
        let header = SnacHeader { family: SnacFamily::Feedbag.as_u16(), subtype: QUERY, flags: 0, request_id: self.next_request_id() };
        self.bos_connection.send_snac(&Snac { header, body: Vec::new() }).await?;
        Ok(())
    }

    /// Adds a buddy to a named group, creating the group server-side first
    /// if it doesn't exist yet. Server is the source of truth — this
    /// optimistically updates local state and relies on the 0x0E "status"
    /// ack to confirm; a real app should reconcile on mismatch.
    pub async fn add_buddy(&mut self, screen_name: &str, group_name: &str) -> Result<(), OscarError> {
        let group_id = self.group_id(group_name).await?;
        let item_id = self.next_feedbag_item_id();

        let item = FeedbagItem { name: screen_name.to_string(), group_id, item_id, class_id: FeedbagItem::CLASS_BUDDY, attributes: Vec::new() };
        let header = SnacHeader { family: SnacFamily::Feedbag.as_u16(), subtype: INSERT_ITEM, flags: 0, request_id: self.next_request_id() };
        let body = item.encode();
        eprintln!("[oscar-rs] -> insert buddy {item:?}: {}", crate::snac::hex_dump(&body));
        self.bos_connection.send_snac(&Snac { header, body }).await?;

        // Optimistic local update — reconciled for real once 0x0E status comes back.
        self.buddies.push(Buddy {
            screen_name: screen_name.to_string(),
            group_name: group_name.to_string(),
            is_online: false,
            is_away: false,
            away_message: None,
            warning_level: 0,
            is_blocked: false,
        });
        Ok(())
    }

    pub async fn remove_buddy(&mut self, screen_name: &str) -> Result<(), OscarError> {
        let Some(existing) = self.feedbag_items.iter().find(|i| i.class_id == FeedbagItem::CLASS_BUDDY && screen_names_match(&i.name, screen_name)).cloned() else {
            return Ok(());
        };

        let header = SnacHeader { family: SnacFamily::Feedbag.as_u16(), subtype: DELETE_ITEM, flags: 0, request_id: self.next_request_id() };
        self.bos_connection.send_snac(&Snac { header, body: existing.encode() }).await?;
        self.buddies.retain(|b| !screen_names_match(&b.screen_name, screen_name));
        Ok(())
    }

    /// Adds a screen name to your block (deny) list — the OSCAR mechanism
    /// behind "blocking" someone: they can no longer see your presence or
    /// message you. Unlike buddies, block-list entries need no group.
    ///
    /// Being *on* the deny list isn't provably sufficient by itself — Open
    /// OSCAR Server's relationship-computation SQL gates deny-list
    /// enforcement behind a separate privacy-mode preference (a
    /// `CLASS_PDINFO` item). This client does not set it: a bare
    /// `CLASS_PDINFO` item with `item_id: 0` and no attributes round-trips
    /// cleanly against a real server, but the exact same item with a
    /// `pdMode` TLV attached — via `INSERT_ITEM` *or* `UPDATE_ITEM`, tried
    /// both ways — hard-disconnects the connection immediately, with zero
    /// signal from the server either time. That isolates the problem to
    /// attaching *any* attribute to this item class, not the specific TLV
    /// guess; Open OSCAR Server's own test suite never exercises a
    /// `Pdinfo` item with attributes either, so this may be an actual gap
    /// in the server rather than something guessing more bytes would fix.
    /// Deny-list membership itself is unaffected and confirmed working —
    /// this is specifically about the separate enforcement toggle.
    pub async fn add_to_block_list(&mut self, screen_name: &str) -> Result<(), OscarError> {
        if self.feedbag_items.iter().any(|i| i.class_id == FeedbagItem::CLASS_DENY && screen_names_match(&i.name, screen_name)) {
            return Ok(());
        }
        let item_id = self.next_feedbag_item_id();
        let item = FeedbagItem { name: screen_name.to_string(), group_id: 0, item_id, class_id: FeedbagItem::CLASS_DENY, attributes: Vec::new() };
        let header = SnacHeader { family: SnacFamily::Feedbag.as_u16(), subtype: INSERT_ITEM, flags: 0, request_id: self.next_request_id() };
        self.bos_connection.send_snac(&Snac { header, body: item.encode() }).await?;
        self.feedbag_items.push(item);
        if let Some(buddy) = self.buddies.iter_mut().find(|b| screen_names_match(&b.screen_name, screen_name)) {
            buddy.is_blocked = true;
        }
        Ok(())
    }

    pub async fn remove_from_block_list(&mut self, screen_name: &str) -> Result<(), OscarError> {
        let Some(existing) = self.feedbag_items.iter().find(|i| i.class_id == FeedbagItem::CLASS_DENY && screen_names_match(&i.name, screen_name)).cloned() else {
            return Ok(());
        };
        let header = SnacHeader { family: SnacFamily::Feedbag.as_u16(), subtype: DELETE_ITEM, flags: 0, request_id: self.next_request_id() };
        self.bos_connection.send_snac(&Snac { header, body: existing.encode() }).await?;
        self.feedbag_items.retain(|i| !(i.class_id == FeedbagItem::CLASS_DENY && screen_names_match(&i.name, screen_name)));
        if let Some(buddy) = self.buddies.iter_mut().find(|b| screen_names_match(&b.screen_name, screen_name)) {
            buddy.is_blocked = false;
        }
        Ok(())
    }

    /// Family 0x13 (Feedbag) frame dispatch — called from `handle_next_frame`.
    pub(crate) async fn handle_feedbag_frame(&mut self, snac: &Snac) -> Result<(), OscarError> {
        match snac.header.subtype {
            REPLY => self.handle_feedbag_reply(&snac.body).await,
            // Per-item ack of a prior insert/update/delete. Body is a run of
            // u16 result codes, one per item in the original request — fine
            // to ignore for a v0.1 given we're already updating optimistically.
            STATUS => Ok(()),
            _ => Ok(()),
        }
    }

    /// Family 0x03 (Buddy) — presence notifications, separate from the
    /// roster itself. Both arrival and departure bodies are a `UserInfo`
    /// block (confirmed against Open OSCAR Server's `wire.TLVUserInfo`) —
    /// *not* a bare name followed by plain TLVs, which is what this used to
    /// assume before that got checked against a real server.
    pub(crate) fn handle_presence_frame(&mut self, snac: &Snac) {
        match snac.header.subtype {
            0x0B => {
                // buddy arrived (online) — the UserInfo block carries their
                // current status flags and warning level directly.
                if let Some((info, _)) = UserInfo::parse(&snac.body) {
                    // Away can show up in either of two places depending on
                    // server: TLV 0x01 "user flags" (u16, bit 0x0020 = away —
                    // the classic cross-client convention) or TLV 0x06
                    // "status" (u32, bit 0x00000001 = away). Check both.
                    let away_via_flags = info
                        .tlvs
                        .get(&0x01)
                        .map(|data| data.len() >= 2 && u16::from_be_bytes([data[0], data[1]]) & 0x0020 != 0)
                        .unwrap_or(false);
                    let away_via_status = info
                        .tlvs
                        .get(&0x06)
                        .map(|data| data.len() >= 4 && u32::from_be_bytes([data[0], data[1], data[2], data[3]]) & 0x0000_0001 != 0)
                        .unwrap_or(false);
                    self.set_online(&info.screen_name, true);
                    self.set_away(&info.screen_name, away_via_flags || away_via_status);
                    self.set_warning_level(&info.screen_name, info.warning_level);
                }
            }
            0x0C => {
                // buddy departed (offline) — same UserInfo shape, we only need the name.
                if let Some((info, _)) = UserInfo::parse(&snac.body) {
                    self.set_online(&info.screen_name, false);
                }
            }
            _ => {}
        }
    }

    async fn handle_feedbag_reply(&mut self, body: &[u8]) -> Result<(), OscarError> {
        // Layout (best-effort — verify against a Wireshark capture of Pidgin
        // logging into your server, same caveat as the rest of this scaffold):
        //   1 byte:  version
        //   2 bytes: item count
        //   N items, back to back (FeedbagItem::parse_all handles this part)
        //   4 bytes: last-modification timestamp (trailing, ignorable on first sync)
        if body.len() <= 3 {
            return Ok(());
        }
        let item_count = u16::from_be_bytes([body[1], body[2]]) as usize;
        let items = FeedbagItem::parse_all(&body[3..]);
        eprintln!(
            "[oscar-rs] feedbag reply: server says {item_count} items, parsed {} — {:?}",
            items.len(),
            items.iter().map(|i| (i.class_id, &i.name, i.group_id, i.item_id)).collect::<Vec<_>>()
        );
        if items.len() != item_count {
            eprintln!("[oscar-rs] *** item count mismatch — parsing likely desynced partway through; raw body: {}", crate::snac::hex_dump(body));
        }

        // Build group-ID -> name lookup first, since buddy items only carry a groupID.
        let mut group_names: HashMap<u16, String> = HashMap::new();
        group_names.insert(0, "Buddies".to_string()); // root/ungrouped fallback
        for item in items.iter().filter(|i| i.class_id == FeedbagItem::CLASS_GROUP) {
            group_names.insert(item.item_id, item.name.clone());
        }

        let denied: std::collections::HashSet<&str> =
            items.iter().filter(|i| i.class_id == FeedbagItem::CLASS_DENY).map(|i| i.name.as_str()).collect();

        self.buddies = items
            .iter()
            .filter(|i| i.class_id == FeedbagItem::CLASS_BUDDY)
            .take(item_count) // sanity bound, in case parsing overshoots
            .map(|i| Buddy {
                screen_name: i.name.clone(),
                group_name: group_names.get(&i.group_id).cloned().unwrap_or_else(|| "Buddies".to_string()),
                is_online: false,
                is_away: false,
                away_message: None,
                warning_level: 0,
                is_blocked: denied.contains(i.name.as_str()),
            })
            .collect();

        self.feedbag_items = items;

        // Ack receipt so the server proceeds — some implementations wait for
        // this before sending anything further.
        let header = SnacHeader { family: SnacFamily::Feedbag.as_u16(), subtype: USE, flags: 0, request_id: self.next_request_id() };
        self.bos_connection.send_snac(&Snac { header, body: Vec::new() }).await?;
        Ok(())
    }

    fn set_online(&mut self, screen_name: &str, online: bool) {
        if let Some(buddy) = self.buddies.iter_mut().find(|b| screen_names_match(&b.screen_name, screen_name)) {
            buddy.is_online = online;
            if !online {
                // Away status is meaningless once offline — reset so stale
                // state doesn't linger and confuse the UI on their next arrival.
                buddy.is_away = false;
            }
        } else {
            eprintln!("[oscar-rs] presence update for {screen_name:?} but no matching buddy found locally");
        }
    }

    fn set_away(&mut self, screen_name: &str, away: bool) {
        if let Some(buddy) = self.buddies.iter_mut().find(|b| screen_names_match(&b.screen_name, screen_name)) {
            buddy.is_away = away;
        }
    }

    /// Also called from `messaging.rs`'s ICBM warning-reply handler, hence `pub(crate)`.
    pub(crate) fn set_warning_level(&mut self, screen_name: &str, level: u16) {
        if let Some(buddy) = self.buddies.iter_mut().find(|b| screen_names_match(&b.screen_name, screen_name)) {
            buddy.warning_level = level;
        }
    }

    async fn group_id(&mut self, name: &str) -> Result<u16, OscarError> {
        if let Some(existing) = self.feedbag_items.iter().find(|i| i.class_id == FeedbagItem::CLASS_GROUP && screen_names_match(&i.name, name)) {
            eprintln!("[oscar-rs] group {name:?} already exists locally (item_id={})", existing.item_id);
            return Ok(existing.item_id);
        }
        // New group: create it too. Real clients send both the group item
        // and the buddy item in one insertItem SNAC with multiple items
        // concatenated; simplified here to two separate calls for clarity.
        let new_group_id = self.next_feedbag_item_id();
        let group_item = FeedbagItem { name: name.to_string(), group_id: 0, item_id: new_group_id, class_id: FeedbagItem::CLASS_GROUP, attributes: Vec::new() };
        let header = SnacHeader { family: SnacFamily::Feedbag.as_u16(), subtype: INSERT_ITEM, flags: 0, request_id: self.next_request_id() };
        let body = group_item.encode();
        eprintln!("[oscar-rs] -> insert group {group_item:?}: {}", crate::snac::hex_dump(&body));
        self.bos_connection.send_snac(&Snac { header, body }).await?;
        self.feedbag_items.push(group_item);
        Ok(new_group_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedbag_item_round_trips() {
        let item = FeedbagItem {
            name: "MyBuddy".to_string(),
            group_id: 1,
            item_id: 5,
            class_id: FeedbagItem::CLASS_BUDDY,
            attributes: vec![Tlv::new(0x01, vec![0xAA, 0xBB])],
        };
        let encoded = item.encode();
        let (parsed, consumed) = FeedbagItem::parse(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(parsed, item);
    }

    /// A real feedbag-reply body, captured (via debug logging) from an
    /// actual Open OSCAR Server instance. Ground truth for the wire format —
    /// this is what caught the previous, wrong "1-byte name length" fix:
    /// under that reading, the second item's name decoded as empty instead
    /// of "catmints", with the real length byte misread as leftover data.
    #[test]
    fn feedbag_item_parse_all_decodes_a_real_server_reply() {
        let body: &[u8] = &[
            0x00, 0x00, 0x02, 0x00, 0x07, 0x42, 0x75, 0x64, 0x64, 0x69, 0x65, 0x73, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x08, 0x63, 0x61, 0x74, 0x6d, 0x69, 0x6e, 0x74, 0x73, 0x00,
            0x02, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x6a, 0x5d, 0x22, 0xd8,
        ];
        // First 3 bytes are version + item count, same slicing handle_feedbag_reply does.
        let items = FeedbagItem::parse_all(&body[3..]);
        assert_eq!(
            items,
            vec![
                FeedbagItem { name: "Buddies".to_string(), group_id: 0, item_id: 2, class_id: FeedbagItem::CLASS_GROUP, attributes: Vec::new() },
                FeedbagItem { name: "catmints".to_string(), group_id: 2, item_id: 3, class_id: FeedbagItem::CLASS_BUDDY, attributes: Vec::new() },
            ]
        );
    }

    #[test]
    fn feedbag_item_parse_all_handles_a_run_of_items() {
        let a = FeedbagItem { name: "Alice".to_string(), group_id: 0, item_id: 1, class_id: FeedbagItem::CLASS_GROUP, attributes: Vec::new() };
        let b = FeedbagItem { name: "Bob".to_string(), group_id: 1, item_id: 2, class_id: FeedbagItem::CLASS_BUDDY, attributes: Vec::new() };
        let mut data = a.encode();
        data.extend(b.encode());

        let items = FeedbagItem::parse_all(&data);
        assert_eq!(items, vec![a, b]);
    }

}
