//! ICBM ("Inter-Client Basic Message", SNAC family 0x04) — instant messages.
//! The historical AOL name has stuck through every implementation since,
//! same as "Feedbag" for the buddy list.

use crate::chat_nav::ChatRoomHandle;
use crate::client::{OscarError, OscarSession};
use crate::snac::{Snac, SnacFamily, SnacHeader, Tlv, UserInfo};

const SEND_IM: u16 = 0x06;
pub(crate) const INCOMING_IM: u16 = 0x07;
const SEND_WARNING: u16 = 0x08;
pub(crate) const WARNING_REPLY: u16 = 0x09;

// Channel-2 (rendezvous) ICBMCh2Fragment layout — used for chat-room
// invites. See research/chat-rooms.md for how each of these was confirmed
// against Open OSCAR Server's actual wire/snacs.go.
const RDV_TYPE_PROPOSE: u16 = 0x00;
const CAP_CHAT: [u8; 16] = [
    0x74, 0x8F, 0x24, 0x20, 0x62, 0x87, 0x11, 0xD1, 0x82, 0x22, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00,
];
const RDV_TLV_SEQ_NUM: u16 = 0x0A;
const RDV_TLV_INVITATION: u16 = 0x0C;
const RDV_TLV_SVC_DATA: u16 = 0x2711;

/// An instant message received from another user.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct IncomingIm {
    pub from: String,
    pub text: String,
}

/// A chat-room invite proposal (ICBM channel 2), distinguished from a
/// regular instant message only by the channel field in the same
/// `ICBMChannelMsgToClient` SNAC — see `icbm_channel`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ChatInvite {
    pub from: String,
    pub invitation_text: String,
    pub room: ChatRoomHandle,
}

/// The cookie+channel+recipient-BUF prefix every ICBM send (channel 1 plain
/// text, or channel 2 rendezvous) shares — factored out of `send_message`
/// without changing its output (still channel 1, byte-identical).
fn icbm_send_prefix(channel: u16, recipient: &str) -> Vec<u8> {
    let mut body = Vec::new();
    let cookie: [u8; 8] = rand::random();
    body.extend_from_slice(&cookie);
    body.extend_from_slice(&channel.to_be_bytes());
    let name_bytes = recipient.as_bytes();
    body.push(name_bytes.len() as u8);
    body.extend_from_slice(name_bytes);
    body
}

/// Reads the 2-byte channel field every incoming ICBM message carries right
/// after the 8-byte cookie (offset 8-9) — present but previously unused by
/// `parse_incoming_im`, which unconditionally assumed channel 1.
pub(crate) fn icbm_channel(body: &[u8]) -> Option<u16> {
    if body.len() < 10 {
        return None;
    }
    Some(u16::from_be_bytes([body[8], body[9]]))
}

/// Best-effort room name extraction from a room cookie's `"{exchange}-{instance}-{name}"`
/// format (confirmed against Open OSCAR Server's own `state.ChatRoom.Cookie()`
/// — not a protocol guarantee, just this server's convention) — an invite's
/// `SvcData` doesn't carry the room name directly, only exchange/cookie/
/// instance, so this is how an invite can show a name before ever joining
/// and getting the official `ChatRoomInfoUpdate`. Never relied on for the
/// actual join, which only ever uses exchange/cookie/instance.
fn room_name_from_cookie(cookie: &str) -> String {
    cookie.splitn(3, '-').nth(2).unwrap_or(cookie).to_string()
}

impl OscarSession {
    /// Sends a plain-text instant message. ICBM send-IM SNAC body layout:
    ///   8 bytes: message "cookie" (client-chosen, echoed back in acks — random is fine)
    ///   2 bytes: channel (1 = plain text)
    ///   BUF: recipient screen name (1-byte length + chars, no type field —
    ///        unlike the rest of the SNAC, which is TLVs)
    ///   TLV 0x02: message data, itself containing nested fragments
    ///     (0x0501 = features, 0x0101 = text)
    pub async fn send_message(&mut self, recipient: &str, text: &str) -> Result<(), OscarError> {
        let mut body = icbm_send_prefix(1, recipient);

        // Message TLV (type 0x02) wraps two inner fragments.
        let mut message_inner = Vec::new();
        // Feature fragment — clients usually send a fixed "capabilities" blob
        // here; an empty/minimal one is tolerated by most permissive OSCAR
        // servers.
        message_inner.extend(Tlv::new(0x0501, vec![0x01, 0x01, 0x01, 0x02]).encode());
        let mut text_fragment = vec![0x00, 0x00]; // charset + charsubset
        text_fragment.extend_from_slice(text.as_bytes());
        message_inner.extend(Tlv::new(0x0101, text_fragment).encode());

        body.extend(Tlv::new(0x02, message_inner).encode());

        let header = SnacHeader { family: SnacFamily::Messaging.as_u16(), subtype: SEND_IM, flags: 0, request_id: self.next_request_id() };
        self.bos_connection.send_snac(&Snac { header, body }).await?;
        Ok(())
    }

    /// Invites a buddy to a chat room — an ICBM channel-2 (rendezvous)
    /// "propose" message, not a Chat/ChatNav SNAC (confirmed no
    /// `ChatSendInvite` handler exists in Open OSCAR Server despite the
    /// subtype constant existing — invites genuinely ride ICBM). Same
    /// SNAC(0x04,0x06) subtype as a regular send, distinguished only by the
    /// channel field and an `ICBMCh2Fragment` (TLV 0x05) instead of a plain
    /// text TLV. The invitee's client parses this and joins directly via
    /// the room's exchange/cookie/instance — no formal ICBM-level accept
    /// reply is needed for that to work, so this client doesn't implement
    /// one.
    pub async fn send_chat_invite(&mut self, recipient: &str, room: &ChatRoomHandle, invitation_text: &str) -> Result<(), OscarError> {
        let mut body = icbm_send_prefix(2, recipient);

        let mut svc_data = room.exchange.to_be_bytes().to_vec();
        let room_cookie_bytes = room.room_cookie.as_bytes();
        svc_data.push(room_cookie_bytes.len() as u8);
        svc_data.extend_from_slice(room_cookie_bytes);
        svc_data.extend_from_slice(&room.instance.to_be_bytes());

        let mut fragment_tlvs = Vec::new();
        fragment_tlvs.extend(Tlv::new(RDV_TLV_SEQ_NUM, 1u16.to_be_bytes().to_vec()).encode());
        fragment_tlvs.extend(Tlv::new(RDV_TLV_INVITATION, invitation_text.as_bytes().to_vec()).encode());
        fragment_tlvs.extend(Tlv::new(RDV_TLV_SVC_DATA, svc_data).encode());

        let mut fragment = RDV_TYPE_PROPOSE.to_be_bytes().to_vec();
        let rendezvous_cookie: [u8; 8] = rand::random(); // distinct from the ICBM message cookie above
        fragment.extend_from_slice(&rendezvous_cookie);
        fragment.extend_from_slice(&CAP_CHAT);
        fragment.extend(fragment_tlvs);

        body.extend(Tlv::new(0x05, fragment).encode());

        let header = SnacHeader { family: SnacFamily::Messaging.as_u16(), subtype: SEND_IM, flags: 0, request_id: self.next_request_id() };
        self.bos_connection.send_snac(&Snac { header, body }).await?;
        Ok(())
    }

    /// Sends an ICBM warning ("evil") to a buddy — the mechanism behind the
    /// classic AIM "Warn" button. Request body: 2-byte flags (bit 0x0001 =
    /// anonymous) + BUF screen name. The reply (subtype 0x09) carries the
    /// target's old/new warning level but no screen name, so attribution
    /// relies on matching the request_id back up — see `handle_warning_reply`.
    pub async fn send_warning(&mut self, screen_name: &str, anonymous: bool) -> Result<(), OscarError> {
        let mut body = Vec::new();
        let flags: u16 = if anonymous { 0x0001 } else { 0 };
        body.extend_from_slice(&flags.to_be_bytes());
        let name_bytes = screen_name.as_bytes();
        body.push(name_bytes.len() as u8);
        body.extend_from_slice(name_bytes);

        let request_id = self.next_request_id();
        let header = SnacHeader { family: SnacFamily::Messaging.as_u16(), subtype: SEND_WARNING, flags: 0, request_id };
        self.bos_connection.send_snac(&Snac { header, body }).await?;
        self.pending_warnings.insert(request_id, screen_name.to_string());
        Ok(())
    }

    /// Family 0x04 subtype 0x09 — reply to a warning we sent. Body: 2-byte
    /// old level, 2-byte new level (both a 0-1000 permille, i.e. percent*10).
    /// Best-effort layout, same caveat as elsewhere in this codebase —
    /// unverified against a real server capture.
    pub(crate) fn handle_warning_reply(&mut self, snac: &Snac) {
        if snac.body.len() < 4 {
            return;
        }
        let new_level = u16::from_be_bytes([snac.body[2], snac.body[3]]);
        if let Some(screen_name) = self.pending_warnings.remove(&snac.header.request_id) {
            self.set_warning_level(&screen_name, new_level);
        }
    }
}

/// Layout: 8-byte cookie, 2-byte channel, then a `UserInfo` block for the
/// sender (confirmed against Open OSCAR Server's
/// `wire.SNAC_0x04_0x07_ICBMChannelMsgToClient`: name + raw warning level +
/// TLV count + TLVs — *not* a bare name directly followed by plain TLVs, a
/// wrong assumption that silently broke incoming-message parsing until
/// checked against a real server), then the message's own TLVs including
/// 0x02 (message data) containing nested fragments.
pub(crate) fn parse_incoming_im(body: &[u8]) -> Option<IncomingIm> {
    if body.len() <= 10 {
        return None;
    }
    let (sender_info, consumed) = UserInfo::parse(&body[10..])?; // skip cookie + channel
    let index = 10 + consumed;
    let sender = sender_info.screen_name;

    let tlvs = Tlv::parse_all(&body[index..]);
    let Some(message_tlv) = tlvs.get(&0x02) else {
        return Some(IncomingIm { from: sender, text: String::new() });
    };

    // Inside the message TLV: nested fragments, each itself type/length/value.
    let fragments = Tlv::parse_all(message_tlv);
    let Some(text_fragment) = fragments.get(&0x0101) else {
        return Some(IncomingIm { from: sender, text: String::new() });
    };
    if text_fragment.len() <= 2 {
        return Some(IncomingIm { from: sender, text: String::new() });
    }
    let text = String::from_utf8_lossy(&text_fragment[2..]).to_string();
    Some(IncomingIm { from: sender, text })
}

/// Same envelope as `parse_incoming_im` (cookie, channel, sender
/// `UserInfo`), but the payload is an `ICBMCh2Fragment` (TLV 0x05) instead
/// of plain text: `Type(u16) + Cookie([8]byte) + Capability([16]byte) +
/// TLVRestBlock`. Only the invitation text (TLV 0x0C) and service data
/// (TLV 0x2711, an `Exchange u16 + Cookie string(len_prefix=u8) + Instance
/// u16` triple) matter for joining — the capability UUID isn't checked
/// against `CAP_CHAT` here since `dispatch_frame` only calls this once it's
/// already routed on channel 2, and this trimmed scope doesn't support any
/// other channel-2 capability (file transfer, etc.) that might also arrive
/// this way.
pub(crate) fn parse_chat_invite(body: &[u8]) -> Option<ChatInvite> {
    if body.len() <= 10 {
        return None;
    }
    let (sender_info, consumed) = UserInfo::parse(&body[10..])?;
    let index = 10 + consumed;
    let from = sender_info.screen_name;

    let tlvs = Tlv::parse_all(&body[index..]);
    let fragment = tlvs.get(&0x05)?;
    const FRAGMENT_HEADER_LEN: usize = 2 + 8 + 16; // Type + Cookie + Capability
    if fragment.len() < FRAGMENT_HEADER_LEN {
        return None;
    }
    let fragment_tlvs = Tlv::parse_all(&fragment[FRAGMENT_HEADER_LEN..]);

    let invitation_text = fragment_tlvs
        .get(&RDV_TLV_INVITATION)
        .map(|v| String::from_utf8_lossy(v).to_string())
        .unwrap_or_default();

    let svc_data = fragment_tlvs.get(&RDV_TLV_SVC_DATA)?;
    if svc_data.len() < 3 {
        return None;
    }
    let exchange = u16::from_be_bytes([svc_data[0], svc_data[1]]);
    let cookie_len = svc_data[2] as usize;
    if svc_data.len() < 3 + cookie_len + 2 {
        return None;
    }
    let room_cookie = String::from_utf8_lossy(&svc_data[3..3 + cookie_len]).to_string();
    let instance = u16::from_be_bytes([svc_data[3 + cookie_len], svc_data[3 + cookie_len + 1]]);

    Some(ChatInvite {
        from,
        invitation_text,
        room: ChatRoomHandle { exchange, room_cookie: room_cookie.clone(), instance, room_name: room_name_from_cookie(&room_cookie) },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_incoming_im_body(sender: &str, text: &str) -> Vec<u8> {
        let mut body = vec![0u8; 8]; // cookie
        body.extend_from_slice(&1u16.to_be_bytes()); // channel

        // Sender UserInfo block: name, then raw warning level, then TLV count (0 here).
        body.push(sender.len() as u8);
        body.extend_from_slice(sender.as_bytes());
        body.extend_from_slice(&0u16.to_be_bytes()); // warning level
        body.extend_from_slice(&0u16.to_be_bytes()); // TLV count

        let mut message_inner = Vec::new();
        let mut text_fragment = vec![0x00, 0x00];
        text_fragment.extend_from_slice(text.as_bytes());
        message_inner.extend(Tlv::new(0x0101, text_fragment).encode());
        body.extend(Tlv::new(0x02, message_inner).encode());
        body
    }

    #[test]
    fn parse_incoming_im_extracts_sender_and_text() {
        let body = build_incoming_im_body("Buddy1", "hello there");
        let im = parse_incoming_im(&body).unwrap();
        assert_eq!(im.from, "Buddy1");
        assert_eq!(im.text, "hello there");
    }

    #[test]
    fn parse_incoming_im_rejects_too_short_body() {
        assert!(parse_incoming_im(&[0u8; 5]).is_none());
    }

    #[test]
    fn icbm_channel_reads_the_channel_field() {
        let channel1 = build_incoming_im_body("Buddy1", "hi");
        assert_eq!(icbm_channel(&channel1), Some(1));
    }

    fn build_chat_invite_body(sender: &str, invitation_text: &str, room: &ChatRoomHandle) -> Vec<u8> {
        let mut body = vec![0u8; 8]; // cookie
        body.extend_from_slice(&2u16.to_be_bytes()); // channel 2

        body.push(sender.len() as u8);
        body.extend_from_slice(sender.as_bytes());
        body.extend_from_slice(&0u16.to_be_bytes()); // warning level
        body.extend_from_slice(&0u16.to_be_bytes()); // TLV count

        let mut svc_data = room.exchange.to_be_bytes().to_vec();
        svc_data.push(room.room_cookie.len() as u8);
        svc_data.extend_from_slice(room.room_cookie.as_bytes());
        svc_data.extend_from_slice(&room.instance.to_be_bytes());

        let mut fragment_tlvs = Vec::new();
        fragment_tlvs.extend(Tlv::new(RDV_TLV_SEQ_NUM, 1u16.to_be_bytes().to_vec()).encode());
        fragment_tlvs.extend(Tlv::new(RDV_TLV_INVITATION, invitation_text.as_bytes().to_vec()).encode());
        fragment_tlvs.extend(Tlv::new(RDV_TLV_SVC_DATA, svc_data).encode());

        let mut fragment = RDV_TYPE_PROPOSE.to_be_bytes().to_vec();
        fragment.extend_from_slice(&[0u8; 8]); // rendezvous cookie
        fragment.extend_from_slice(&CAP_CHAT);
        fragment.extend(fragment_tlvs);

        body.extend(Tlv::new(0x05, fragment).encode());
        body
    }

    #[test]
    fn parses_a_chat_invite_including_room_name_from_cookie() {
        let room = ChatRoomHandle { exchange: 4, room_cookie: "4-0-MyRoom".into(), instance: 0, room_name: String::new() };
        let body = build_chat_invite_body("Alice", "join us!", &room);

        assert_eq!(icbm_channel(&body), Some(2));
        let invite = parse_chat_invite(&body).unwrap();
        assert_eq!(invite.from, "Alice");
        assert_eq!(invite.invitation_text, "join us!");
        assert_eq!(invite.room.exchange, 4);
        assert_eq!(invite.room.room_cookie, "4-0-MyRoom");
        assert_eq!(invite.room.instance, 0);
        assert_eq!(invite.room.room_name, "MyRoom");
    }

    #[test]
    fn room_name_from_cookie_handles_a_room_name_containing_dashes() {
        // splitn(3, '-') means a room named "my-cool-room" doesn't get
        // truncated at the first dash.
        assert_eq!(room_name_from_cookie("4-0-my-cool-room"), "my-cool-room");
    }
}
