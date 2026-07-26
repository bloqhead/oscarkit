//! ChatNav (SNAC family 0x0D) — an ephemeral connection used only to create
//! a chat room. Unlike BOS or a joined room's Chat connection, nothing here
//! is long-lived: connect, create the room, drop the connection. The actual
//! chatting happens on a separate, dedicated Chat (0x0E) connection per
//! room — see `chat.rs`.
//!
//! This trimmed implementation only supports creating a private
//! (create-on-demand) room, not browsing/joining an existing public one —
//! see `research/chat-rooms.md` for the full protocol and why the narrower
//! scope was chosen. Exchange `4` (`state.PrivateExchange` in Open OSCAR
//! Server's own source) is a stable protocol constant, not something that
//! needs discovering via `ChatNavRequestChatRights` first.

use crate::client::{connect_redirect, OscarError};
use crate::flap::FlapChannel;
use crate::server_address::ServerAddress;
use crate::snac::{Snac, SnacFamily, SnacHeader, Tlv};

const CREATE_ROOM: u16 = 0x08;
const NAV_INFO: u16 = 0x09;

const PRIVATE_EXCHANGE: u16 = 4;
const ROOM_TLV_NAME: u16 = 0xD3;
const NAV_TLV_ROOM_INFO: u16 = 0x04;

/// Identifies a specific chat room — returned by `create_room` (with the
/// server-assigned cookie) and carried in an invite's payload for the
/// invitee to join without ever touching ChatNav themselves.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ChatRoomHandle {
    pub exchange: u16,
    pub room_cookie: String,
    pub instance: u16,
    pub room_name: String,
}

/// Parses a `SNAC_0x0E_0x02_ChatRoomInfoUpdate` body (`Exchange u16; Cookie
/// string(len_prefix=u8); InstanceNumber u16; DetailLevel u8; TLVBlock`) —
/// used both for ChatNav's create-room reply (wrapped in a TLV) and, later,
/// directly on the Chat connection's own room-info-update frames.
pub(crate) fn parse_chat_room_info_update(body: &[u8]) -> Option<ChatRoomHandle> {
    if body.len() < 5 {
        return None;
    }
    let exchange = u16::from_be_bytes([body[0], body[1]]);
    let cookie_len = body[2] as usize;
    if body.len() < 3 + cookie_len + 3 {
        return None;
    }
    let room_cookie = String::from_utf8_lossy(&body[3..3 + cookie_len]).to_string();
    let mut index = 3 + cookie_len;
    let instance = u16::from_be_bytes([body[index], body[index + 1]]);
    index += 2;
    // DetailLevel (1 byte) follows, then a TLV *count* + that many TLVs
    // (same TLVBlock shape `UserInfo`/`FeedbagItem` already use elsewhere).
    index += 1;
    if body.len() < index + 2 {
        return None;
    }
    let tlv_count = u16::from_be_bytes([body[index], body[index + 1]]) as usize;
    index += 2;
    let (tlvs, _) = Tlv::parse_n(&body[index..], tlv_count);
    let room_name = tlvs
        .get(&ROOM_TLV_NAME)
        .map(|v| String::from_utf8_lossy(v).to_string())
        .unwrap_or_default();

    Some(ChatRoomHandle { exchange, room_cookie, instance, room_name })
}

pub(crate) struct ChatNavSession {
    connection: crate::connection::FlapConnection,
    ids: crate::client::RequestIdCounter,
}

impl ChatNavSession {
    /// Connects to the ChatNav redirect target obtained via
    /// `OscarSession::request_service_redirect`. No `ClientOnline`
    /// announcement is needed here (confirmed against Open OSCAR Server's
    /// `foodgroup/chat_nav.go`: none of its handlers gate on prior
    /// session-online state, unlike BOS's Feedbag sync).
    pub(crate) async fn connect(address: &ServerAddress, cookie: Vec<u8>) -> Result<Self, OscarError> {
        let connection = connect_redirect(address, cookie).await?;
        Ok(ChatNavSession { connection, ids: crate::client::RequestIdCounter::new() })
    }

    /// SNAC(0x0D,0x08) `ChatNavCreateRoom` — reuses the Chat family's
    /// `ChatRoomInfoUpdate` struct shape: `Exchange=4; Cookie="create";
    /// InstanceNumber=0; DetailLevel=0x02` (the server ignores the
    /// client-sent DetailLevel when creating and always returns its own
    /// hardcoded `0x02` — sent anyway for convention) `; TLVBlock{0xD3=room
    /// name}`. Waits for SNAC(0x0D,0x09) `ChatNavNavInfo`, unwraps its TLV
    /// 0x04 to get the real server-assigned room cookie.
    pub(crate) async fn create_room(&mut self, room_name: &str) -> Result<ChatRoomHandle, OscarError> {
        let mut body = PRIVATE_EXCHANGE.to_be_bytes().to_vec();
        let cookie_bytes = b"create";
        body.push(cookie_bytes.len() as u8);
        body.extend_from_slice(cookie_bytes);
        body.extend_from_slice(&0u16.to_be_bytes()); // InstanceNumber
        body.push(0x02); // DetailLevel
        let name_tlv = Tlv::new(ROOM_TLV_NAME, room_name.as_bytes().to_vec());
        body.extend_from_slice(&1u16.to_be_bytes()); // TLV count
        body.extend(name_tlv.encode());

        let header = SnacHeader { family: SnacFamily::ChatNav.as_u16(), subtype: CREATE_ROOM, flags: 0, request_id: self.ids.next() };
        self.connection.send_snac(&Snac { header, body }).await?;

        loop {
            let frame = self
                .connection
                .read_frame()
                .await?
                .ok_or(OscarError::ConnectionClosed("waiting for ChatNavNavInfo"))?;
            if frame.channel != FlapChannel::Data {
                continue;
            }
            let Some(snac) = Snac::parse(&frame.payload) else { continue };
            if snac.header.family == SnacFamily::ChatNav.as_u16() && snac.header.subtype == NAV_INFO {
                let tlvs = Tlv::parse_all(&snac.body);
                let room_info_bytes = tlvs
                    .get(&NAV_TLV_ROOM_INFO)
                    .ok_or(OscarError::UnexpectedResponse("ChatNavNavInfo missing room info (TLV 0x04)"))?;
                return parse_chat_room_info_update(room_info_bytes)
                    .ok_or(OscarError::UnexpectedResponse("malformed ChatRoomInfoUpdate in ChatNavNavInfo"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_room_info_update(exchange: u16, cookie: &str, instance: u16, room_name: &str) -> Vec<u8> {
        let mut body = exchange.to_be_bytes().to_vec();
        body.push(cookie.len() as u8);
        body.extend_from_slice(cookie.as_bytes());
        body.extend_from_slice(&instance.to_be_bytes());
        body.push(0x02); // DetailLevel
        let name_tlv = Tlv::new(ROOM_TLV_NAME, room_name.as_bytes().to_vec());
        body.extend_from_slice(&1u16.to_be_bytes());
        body.extend(name_tlv.encode());
        body
    }

    #[test]
    fn parses_a_chat_room_info_update() {
        let body = build_room_info_update(4, "4-0-MyRoom", 0, "MyRoom");
        let handle = parse_chat_room_info_update(&body).unwrap();
        assert_eq!(handle.exchange, 4);
        assert_eq!(handle.room_cookie, "4-0-MyRoom");
        assert_eq!(handle.instance, 0);
        assert_eq!(handle.room_name, "MyRoom");
    }
}
