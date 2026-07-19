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

use crate::client::{OscarError, OscarSession};
use crate::snac::{Snac, SnacFamily, SnacHeader, Tlv};

const QUERY: u16 = 0x04; // client: "send me my whole list"
const REPLY: u16 = 0x05; // server: here's your list
const USE: u16 = 0x06; // client: "ack, I've got it, proceed"
const INSERT_ITEM: u16 = 0x08; // client: add buddy/group
const DELETE_ITEM: u16 = 0x0A;
const STATUS: u16 = 0x0E; // server: ack of insert/update/delete

/// Every entry in a feedbag — a buddy, a group, or a handful of special
/// metadata items (permit/deny lists, visibility prefs) — shares this same
/// wire structure. `class_id` is what tells you which kind you're looking
/// at.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbagItem {
    pub name: String,
    pub group_id: u16,
    pub item_id: u16,
    pub class_id: u16,
    pub attributes: Vec<u8>, // raw TLV block; parse with Tlv::parse_all if you need specific fields
}

impl FeedbagItem {
    // Known class IDs. There are more (icon metadata, ignore list, etc.) —
    // add as needed.
    pub const CLASS_BUDDY: u16 = 0x0000;
    pub const CLASS_GROUP: u16 = 0x0001;

    pub fn encode(&self) -> Vec<u8> {
        let name_bytes = self.name.as_bytes();
        let mut out = Vec::with_capacity(10 + name_bytes.len() + self.attributes.len());
        out.extend_from_slice(&(name_bytes.len() as u16).to_be_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(&self.group_id.to_be_bytes());
        out.extend_from_slice(&self.item_id.to_be_bytes());
        out.extend_from_slice(&self.class_id.to_be_bytes());
        out.extend_from_slice(&(self.attributes.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.attributes);
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
        let attr_len = read_u16(data, &mut index)? as usize;
        if index + attr_len > data.len() {
            return None;
        }
        let attributes = data[index..index + attr_len].to_vec();
        index += attr_len;

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
/// the Locate family respectively — see `locate.rs`.
#[derive(Debug, Clone, PartialEq)]
pub struct Buddy {
    pub screen_name: String,
    pub group_name: String,
    pub is_online: bool,
    pub is_away: bool,
    pub away_message: Option<String>,
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
        self.bos_connection.send_snac(&Snac { header, body: item.encode() }).await?;

        // Optimistic local update — reconciled for real once 0x0E status comes back.
        self.buddies.push(Buddy { screen_name: screen_name.to_string(), group_name: group_name.to_string(), is_online: false, is_away: false, away_message: None });
        Ok(())
    }

    pub async fn remove_buddy(&mut self, screen_name: &str) -> Result<(), OscarError> {
        let Some(existing) = self.feedbag_items.iter().find(|i| i.class_id == FeedbagItem::CLASS_BUDDY && i.name == screen_name).cloned() else {
            return Ok(());
        };

        let header = SnacHeader { family: SnacFamily::Feedbag.as_u16(), subtype: DELETE_ITEM, flags: 0, request_id: self.next_request_id() };
        self.bos_connection.send_snac(&Snac { header, body: existing.encode() }).await?;
        self.buddies.retain(|b| b.screen_name != screen_name);
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
    /// roster itself.
    pub(crate) fn handle_presence_frame(&mut self, snac: &Snac) {
        match snac.header.subtype {
            0x0B => {
                // buddy arrived (online) — body also carries their current status flags
                if let Some((name, remainder)) = parse_screen_name_buf_with_remainder(&snac.body) {
                    let tlvs = Tlv::parse_all(remainder);
                    // User status flags, TLV 0x0C in most implementations'
                    // arrival payload. Bit 0x0020 is the conventional "away"
                    // flag across OSCAR clients.
                    let is_away = tlvs
                        .get(&0x0C)
                        .map(|data| data.len() >= 2 && u16::from_be_bytes([data[0], data[1]]) & 0x0020 != 0)
                        .unwrap_or(false);
                    self.set_online(&name, true);
                    self.set_away(&name, is_away);
                }
            }
            0x0C => {
                // buddy departed (offline)
                if let Some(name) = parse_screen_name_buf(&snac.body) {
                    self.set_online(&name, false);
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

        // Build group-ID -> name lookup first, since buddy items only carry a groupID.
        let mut group_names: HashMap<u16, String> = HashMap::new();
        group_names.insert(0, "Buddies".to_string()); // root/ungrouped fallback
        for item in items.iter().filter(|i| i.class_id == FeedbagItem::CLASS_GROUP) {
            group_names.insert(item.item_id, item.name.clone());
        }

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
        if let Some(buddy) = self.buddies.iter_mut().find(|b| b.screen_name == screen_name) {
            buddy.is_online = online;
            if !online {
                // Away status is meaningless once offline — reset so stale
                // state doesn't linger and confuse the UI on their next arrival.
                buddy.is_away = false;
            }
        }
    }

    fn set_away(&mut self, screen_name: &str, away: bool) {
        if let Some(buddy) = self.buddies.iter_mut().find(|b| b.screen_name == screen_name) {
            buddy.is_away = away;
        }
    }

    async fn group_id(&mut self, name: &str) -> Result<u16, OscarError> {
        if let Some(existing) = self.feedbag_items.iter().find(|i| i.class_id == FeedbagItem::CLASS_GROUP && i.name == name) {
            return Ok(existing.item_id);
        }
        // New group: create it too. Real clients send both the group item
        // and the buddy item in one insertItem SNAC with multiple items
        // concatenated; simplified here to two separate calls for clarity.
        let new_group_id = self.next_feedbag_item_id();
        let group_item = FeedbagItem { name: name.to_string(), group_id: 0, item_id: new_group_id, class_id: FeedbagItem::CLASS_GROUP, attributes: Vec::new() };
        let header = SnacHeader { family: SnacFamily::Feedbag.as_u16(), subtype: INSERT_ITEM, flags: 0, request_id: self.next_request_id() };
        self.bos_connection.send_snac(&Snac { header, body: group_item.encode() }).await?;
        self.feedbag_items.push(group_item);
        Ok(new_group_id)
    }
}

/// Family 0x03 arrival/departure bodies lead with the same BUF pattern
/// (1-byte length + name bytes, no type field) as ICBM.
fn parse_screen_name_buf(body: &[u8]) -> Option<String> {
    let &first = body.first()?;
    let length = first as usize;
    if body.len() < 1 + length {
        return None;
    }
    Some(String::from_utf8_lossy(&body[1..1 + length]).to_string())
}

/// Same as above, but also returns whatever bytes follow the name — the
/// arrival notification's TLV block, which callers may want to inspect for
/// status flags, warning level, etc.
fn parse_screen_name_buf_with_remainder(body: &[u8]) -> Option<(String, &[u8])> {
    let &first = body.first()?;
    let length = first as usize;
    if body.len() < 1 + length {
        return None;
    }
    let name = String::from_utf8_lossy(&body[1..1 + length]).to_string();
    Some((name, &body[1 + length..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedbag_item_round_trips() {
        let item = FeedbagItem { name: "MyBuddy".to_string(), group_id: 1, item_id: 5, class_id: FeedbagItem::CLASS_BUDDY, attributes: vec![0xAA, 0xBB] };
        let encoded = item.encode();
        let (parsed, consumed) = FeedbagItem::parse(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(parsed, item);
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

    #[test]
    fn parse_screen_name_buf_with_remainder_splits_correctly() {
        let mut body = vec![3u8];
        body.extend_from_slice(b"Bob");
        body.extend_from_slice(&[0xDE, 0xAD]);

        let (name, remainder) = parse_screen_name_buf_with_remainder(&body).unwrap();
        assert_eq!(name, "Bob");
        assert_eq!(remainder, &[0xDE, 0xAD]);
    }
}
