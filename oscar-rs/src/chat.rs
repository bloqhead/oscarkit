//! Chat (SNAC family 0x0E) — the connection for an actual joined room, one
//! per room, separate from and alongside the main BOS connection. A
//! `ChatRoomSession` is a parallel sibling to `OscarSession`, not a field on
//! it: nothing about its connection lifecycle, dispatch, or state belongs
//! nested inside the BOS session.

use crate::client::{screen_names_match, OscarError, RequestIdCounter};
use crate::chat_nav::{parse_chat_room_info_update, ChatRoomHandle};
use crate::connection::{FlapReader, FlapWriter};
use crate::flap::{FlapChannel, FlapFrame};
use crate::snac::{hex_dump, Snac, SnacFamily, SnacHeader, Tlv, UserInfo};

const ROOM_INFO_UPDATE: u16 = 0x02;
const USERS_JOINED: u16 = 0x03;
const USERS_LEFT: u16 = 0x04;
const CHANNEL_MSG_TO_HOST: u16 = 0x05;
const CHANNEL_MSG_TO_CLIENT: u16 = 0x06;

const MSG_TLV_INFO: u16 = 0x05;
const MSG_TLV_TEXT: u16 = 0x01;

/// One other person in the room. Deliberately a small projection (not the
/// full `UserInfo`, which carries a raw TLV map that isn't meaningful to
/// the frontend) — same relationship `Buddy` has to `FeedbagItem`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ChatOccupant {
    pub screen_name: String,
    pub warning_level: u16,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ChatMessage {
    pub from: String,
    pub text: String,
}

pub struct ChatRoomSession {
    writer: FlapWriter,
    reader: Option<FlapReader>,
    ids: RequestIdCounter,
    pub handle: ChatRoomHandle,
    pub occupants: Vec<ChatOccupant>,
    pub messages: Vec<ChatMessage>,
    pub my_screen_name: String,
}

impl ChatRoomSession {
    pub(crate) fn new(writer: FlapWriter, reader: FlapReader, handle: ChatRoomHandle, my_screen_name: String) -> Self {
        ChatRoomSession {
            writer,
            reader: Some(reader),
            ids: RequestIdCounter::new(),
            handle,
            occupants: Vec::new(),
            messages: Vec::new(),
            my_screen_name,
        }
    }

    /// Same one-shot contract as `OscarSession::split_reader` — hands out
    /// the read half so a caller (the Tauri chat actor) can run it on a
    /// dedicated task. Panics if called twice.
    pub fn split_reader(&mut self) -> FlapReader {
        self.reader.take().expect("split_reader() called twice on the same ChatRoomSession")
    }

    pub async fn handle_next_frame(&mut self) -> Result<(), OscarError> {
        let reader = self.reader.as_mut().expect("reader missing — was split_reader() already called?");
        let frame = reader.read_frame().await?.ok_or(OscarError::ConnectionClosed("chat room session"))?;
        self.dispatch_frame(frame).await
    }

    /// Dispatches one family-0x0E frame — room metadata refresh, occupant
    /// join/leave, or an incoming message. Public (like
    /// `OscarSession::dispatch_frame`) so a caller running its own read loop
    /// via `split_reader()` can feed frames in directly.
    pub async fn dispatch_frame(&mut self, frame: FlapFrame) -> Result<(), OscarError> {
        if frame.channel != FlapChannel::Data {
            return Ok(());
        }
        let Some(snac) = Snac::parse(&frame.payload) else {
            eprintln!("[oscar-rs] dropped an unparseable chat FLAP data frame ({} bytes)", frame.payload.len());
            return Ok(());
        };
        if SnacFamily::from_u16(snac.header.family) != Some(SnacFamily::Chat) {
            eprintln!("[oscar-rs] unexpected non-chat family 0x{:04x} on a chat connection — ignored", snac.header.family);
            return Ok(());
        }

        match snac.header.subtype {
            ROOM_INFO_UPDATE => {
                if let Some(updated) = parse_chat_room_info_update(&snac.body) {
                    self.handle = updated;
                }
            }
            USERS_JOINED => {
                for occupant in parse_user_info_list(&snac.body) {
                    if !self.occupants.iter().any(|o| screen_names_match(&o.screen_name, &occupant.screen_name)) {
                        self.occupants.push(occupant);
                    }
                }
            }
            USERS_LEFT => {
                let left = parse_user_info_list(&snac.body);
                self.occupants.retain(|o| !left.iter().any(|l| screen_names_match(&l.screen_name, &o.screen_name)));
            }
            CHANNEL_MSG_TO_CLIENT => {
                if let Some(message) = parse_chat_message(&snac.body) {
                    self.messages.push(message);
                }
            }
            other => eprintln!("[oscar-rs] no handler for chat subtype 0x{other:02x} — ignored: {}", hex_dump(&snac.body)),
        }
        Ok(())
    }

    /// SNAC(0x0E,0x05) `ChatChannelMsgToHost` — own random 8-byte message
    /// cookie (unrelated to the room cookie string), Channel=1, TLV 0x05
    /// wrapping sub-TLV 0x01 (text). Deliberately not setting
    /// `ChatTLVEnableReflectionFlag`: confirmed against Open OSCAR Server's
    /// `foodgroup/chat.go` that the server excludes the sender from its own
    /// broadcast (`RelayToAllExcept`), so this client relies on the caller
    /// pushing the sent message into `messages` itself (optimistic, same
    /// pattern `messaging.rs::send_message`'s callers already use for
    /// 1-on-1 IMs, which the backend also never echoes).
    pub async fn send_message(&mut self, text: &str) -> Result<(), OscarError> {
        let cookie: [u8; 8] = rand::random();
        let mut body = cookie.to_vec();
        body.extend_from_slice(&1u16.to_be_bytes()); // Channel

        let text_tlv = Tlv::new(MSG_TLV_TEXT, text.as_bytes().to_vec());
        let message_info = Tlv::new(MSG_TLV_INFO, text_tlv.encode());
        body.extend(message_info.encode());

        let header = SnacHeader { family: SnacFamily::Chat.as_u16(), subtype: CHANNEL_MSG_TO_HOST, flags: 0, request_id: self.ids.next() };
        self.writer.send_snac(&Snac { header, body }).await?;
        Ok(())
    }
}

/// `ChatUsersJoined`/`ChatUsersLeft` bodies are a run of `UserInfo` blocks
/// with no count prefix (confirmed against the wire struct — a plain
/// `Users []TLVUserInfo` field with no `count_prefix` tag) — parse
/// repeatedly until the body is exhausted, same idiom already established
/// for other unbounded TLV/block runs in this codebase.
pub(crate) fn parse_user_info_list(body: &[u8]) -> Vec<ChatOccupant> {
    let mut occupants = Vec::new();
    let mut offset = 0;
    while offset < body.len() {
        let Some((info, consumed)) = UserInfo::parse(&body[offset..]) else { break };
        if consumed == 0 {
            break;
        }
        occupants.push(ChatOccupant { screen_name: info.screen_name, warning_level: info.warning_level });
        offset += consumed;
    }
    occupants
}

/// `ChatChannelMsgToClient{Cookie u64; Channel u16; TLVRestBlock}` — the
/// TLVRestBlock carries TLV 0x03 (sender `UserInfo`) and TLV 0x05 (message
/// info, wrapping sub-TLV 0x01 text) per `foodgroup/chat.go`'s
/// `newChatTLVBlock`.
fn parse_chat_message(body: &[u8]) -> Option<ChatMessage> {
    if body.len() < 10 {
        return None;
    }
    let tlvs = Tlv::parse_all(&body[10..]);

    let sender_bytes = tlvs.get(&0x03)?;
    let (sender, _) = UserInfo::parse(sender_bytes)?;

    let message_info = tlvs.get(&MSG_TLV_INFO)?;
    let inner = Tlv::parse_all(message_info);
    let text_bytes = inner.get(&MSG_TLV_TEXT)?;
    let text = String::from_utf8_lossy(text_bytes).to_string();

    Some(ChatMessage { from: sender.screen_name, text })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_user_info(name: &str, warning: u16) -> Vec<u8> {
        let mut data = vec![name.len() as u8];
        data.extend_from_slice(name.as_bytes());
        data.extend_from_slice(&warning.to_be_bytes());
        data.extend_from_slice(&0u16.to_be_bytes()); // TLV count
        data
    }

    #[test]
    fn parses_a_users_joined_body_with_multiple_occupants() {
        let mut body = build_user_info("Alice", 0);
        body.extend(build_user_info("Bob", 50));
        let occupants = parse_user_info_list(&body);
        assert_eq!(occupants.len(), 2);
        assert_eq!(occupants[0].screen_name, "Alice");
        assert_eq!(occupants[1].screen_name, "Bob");
        assert_eq!(occupants[1].warning_level, 50);
    }

    #[test]
    fn parses_a_chat_message() {
        let mut body = vec![0u8; 8]; // message cookie
        body.extend_from_slice(&1u16.to_be_bytes()); // channel

        let sender_info = build_user_info("Alice", 0);
        body.extend(Tlv::new(0x03, sender_info).encode());

        let text_tlv = Tlv::new(MSG_TLV_TEXT, b"hello room".to_vec());
        body.extend(Tlv::new(MSG_TLV_INFO, text_tlv.encode()).encode());

        let message = parse_chat_message(&body).unwrap();
        assert_eq!(message.from, "Alice");
        assert_eq!(message.text, "hello room");
    }
}
