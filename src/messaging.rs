//! ICBM ("Inter-Client Basic Message", SNAC family 0x04) — instant messages.
//! The historical AOL name has stuck through every implementation since,
//! same as "Feedbag" for the buddy list.

use crate::client::{OscarError, OscarSession};
use crate::snac::{Snac, SnacFamily, SnacHeader, Tlv};

const SEND_IM: u16 = 0x06;
pub(crate) const INCOMING_IM: u16 = 0x07;

/// An instant message received from another user.
#[derive(Debug, Clone, PartialEq)]
pub struct IncomingIm {
    pub from: String,
    pub text: String,
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
        let mut body = Vec::new();
        let cookie: [u8; 8] = rand::random();
        body.extend_from_slice(&cookie);
        body.extend_from_slice(&1u16.to_be_bytes()); // channel 1

        let name_bytes = recipient.as_bytes();
        body.push(name_bytes.len() as u8);
        body.extend_from_slice(name_bytes);

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
}

/// Layout: 8-byte cookie, 2-byte channel, then a BUF (1-byte length + name),
/// then TLVs including 0x02 (message data) containing nested fragments.
pub(crate) fn parse_incoming_im(body: &[u8]) -> Option<IncomingIm> {
    if body.len() <= 11 {
        return None;
    }
    let mut index = 10usize; // skip cookie + channel
    let name_length = body[index] as usize;
    index += 1;
    if index + name_length > body.len() {
        return None;
    }
    let sender = String::from_utf8_lossy(&body[index..index + name_length]).to_string();
    index += name_length;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn build_incoming_im_body(sender: &str, text: &str) -> Vec<u8> {
        let mut body = vec![0u8; 8]; // cookie
        body.extend_from_slice(&1u16.to_be_bytes()); // channel
        body.push(sender.len() as u8);
        body.extend_from_slice(sender.as_bytes());

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
}
