//! The Locate family (SNAC 0x02) is OSCAR's mechanism for both user profiles
//! and away messages — they're the same underlying concept ("info about a
//! user that gets fetched on demand"), just different TLV slots in the same
//! SET_INFO / USER_INFO_REPLY structures.
//!
//! The quirk worth internalizing: there's no dedicated "go away" or "come
//! back" command. Setting your away message *is* going away. Sending a
//! SET_INFO with an empty away TLV *is* coming back. The presence system
//! (family 0x03, see `feedbag.rs`) picks up the resulting status-bit change
//! and broadcasts it to your buddies automatically — you don't separately
//! announce "I'm away" beyond setting the message itself.

use crate::client::{screen_names_match, OscarError, OscarSession};
use crate::snac::{Snac, SnacFamily, SnacHeader, Tlv, UserInfo};

const SET_INFO: u16 = 0x04; // client: set my profile/away message
const USER_INFO_QUERY: u16 = 0x05; // client: "tell me about this buddy"
const USER_INFO_REPLY: u16 = 0x06; // server: here's their info

// TLV types used inside both SET_INFO (outgoing) and USER_INFO_REPLY (incoming).
const AWAY_ENCODING: u16 = 0x03;
const AWAY_TEXT: u16 = 0x04;

// Request-type bitmask for USER_INFO_QUERY (confirmed against Open OSCAR
// Server's wire.LocateType* constants — NOT a TLV, a raw field, see below).
const REQUEST_TYPE_UNAVAILABLE: u16 = 0x0002; // "give me their away message"

impl OscarSession {
    /// Sets (or clears, if `None`) your away message. This is the *only*
    /// away mechanism in OSCAR — there's no separate "toggle away mode" —
    /// sending non-empty text here is what makes you appear away to
    /// buddies; sending `None` sends an empty TLV, which is how you come
    /// back.
    pub async fn set_away_message(&mut self, text: Option<&str>) -> Result<(), OscarError> {
        let mut body = Vec::new();
        // Encoding TLVs use a fixed charset string, same convention as
        // message fragments elsewhere in the protocol.
        body.extend(Tlv::new(AWAY_ENCODING, b"us-ascii".to_vec()).encode());
        body.extend(Tlv::new(AWAY_TEXT, text.unwrap_or("").as_bytes().to_vec()).encode());

        let header = SnacHeader { family: SnacFamily::Locate.as_u16(), subtype: SET_INFO, flags: 0, request_id: self.next_request_id() };
        self.bos_connection.send_snac(&Snac { header, body }).await?;

        // Optimistic local update — this is *your own* state, so there's no
        // server round-trip needed to know it took effect the way there is
        // for e.g. buddy list inserts.
        self.away_message = text.map(str::to_string);
        Ok(())
    }

    /// Requests a buddy's current profile/away message. Reply arrives async
    /// via `handle_locate_frame` and updates the matching entry in `buddies`.
    ///
    /// Wire format confirmed against Open OSCAR Server's
    /// `wire.SNAC_0x02_0x05_LocateUserInfoQuery`: this is *not* TLVs at
    /// all — a raw 2-byte request-type bitmask comes first, then a BUF
    /// screen name. (The previous version got both the framing and the
    /// requested bit wrong — it TLV-wrapped both fields and asked for
    /// `0x0001`, the *profile* bit, not `0x0002`, the away-message one.)
    pub async fn request_user_info(&mut self, screen_name: &str) -> Result<(), OscarError> {
        let mut body = REQUEST_TYPE_UNAVAILABLE.to_be_bytes().to_vec();
        let name_bytes = screen_name.as_bytes();
        body.push(name_bytes.len() as u8);
        body.extend_from_slice(name_bytes);

        let header = SnacHeader { family: SnacFamily::Locate.as_u16(), subtype: USER_INFO_QUERY, flags: 0, request_id: self.next_request_id() };
        self.bos_connection.send_snac(&Snac { header, body }).await?;
        Ok(())
    }

    /// Family 0x02 (Locate) frame dispatch — called from `handle_next_frame`.
    ///
    /// The reply (`wire.SNAC_0x02_0x06_LocateUserInfoReply`) is a `UserInfo`
    /// block (name + raw warning level + TLV count + TLVs — see
    /// `snac::UserInfo`) followed by a *separate* plain TLV run carrying the
    /// actual profile/away-message data. Previously this assumed the name
    /// was directly followed by that second TLV run, which — same bug class
    /// as the presence and incoming-message parsing — silently corrupted
    /// the offset once a real server was involved.
    pub(crate) fn handle_locate_frame(&mut self, snac: &Snac) {
        if snac.header.subtype != USER_INFO_REPLY {
            return;
        }

        let Some((info, consumed)) = UserInfo::parse(&snac.body) else { return };

        let tlvs = Tlv::parse_all(&snac.body[consumed..]);
        let away_text = tlvs.get(&AWAY_TEXT).map(|v| String::from_utf8_lossy(v).to_string());

        if let Some(buddy) = self.buddies.iter_mut().find(|b| screen_names_match(&b.screen_name, &info.screen_name)) {
            buddy.away_message = away_text.filter(|t| !t.is_empty());
        }
    }
}
