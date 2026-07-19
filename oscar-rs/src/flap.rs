//! FLAP is the lowest-level framing protocol OSCAR runs on top of. Every
//! single thing sent over the wire — login, IMs, buddy list updates — is
//! wrapped in a FLAP frame first.
//!
//! Wire format (big-endian):
//!   byte 0:      0x2a  (magic "asterisk" marker — every frame starts with this)
//!   byte 1:      channel (see FlapChannel below)
//!   bytes 2-3:   sequence number (u16, client and server each keep their own counter)
//!   bytes 4-5:   payload length (u16)
//!   bytes 6...:  payload (length bytes, meaning depends on channel)

pub const FLAP_MAGIC_BYTE: u8 = 0x2a;
pub const FLAP_HEADER_SIZE: usize = 6;

/// FLAP channels multiplex different kinds of traffic over the same TCP connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlapChannel {
    NewConnection = 0x01, // connection negotiation / hello / disconnect notices
    Data = 0x02,          // SNAC-wrapped data — this is where almost everything lives
    Error = 0x03,
    CloseConnection = 0x04,
    KeepAlive = 0x05,
}

impl FlapChannel {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(FlapChannel::NewConnection),
            0x02 => Some(FlapChannel::Data),
            0x03 => Some(FlapChannel::Error),
            0x04 => Some(FlapChannel::CloseConnection),
            0x05 => Some(FlapChannel::KeepAlive),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlapFrame {
    pub channel: FlapChannel,
    pub sequence: u16,
    pub payload: Vec<u8>,
}

impl FlapFrame {
    /// Serializes this frame to bytes ready to write to the socket.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(FLAP_HEADER_SIZE + self.payload.len());
        out.push(FLAP_MAGIC_BYTE);
        out.push(self.channel as u8);
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&(self.payload.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.payload);
        out
    }

    /// Parses just the 6-byte header to learn how many more bytes to read
    /// for the payload. Returns None if the header doesn't start with the
    /// magic byte (out of sync / garbage) or the channel byte is unrecognized.
    pub fn parse_header(header: &[u8]) -> Option<(FlapChannel, u16, u16)> {
        if header.len() != FLAP_HEADER_SIZE || header[0] != FLAP_MAGIC_BYTE {
            return None;
        }
        let channel = FlapChannel::from_u8(header[1])?;
        let sequence = u16::from_be_bytes([header[2], header[3]]);
        let length = u16::from_be_bytes([header[4], header[5]]);
        Some((channel, sequence, length))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_then_parse_header_round_trips() {
        let frame = FlapFrame {
            channel: FlapChannel::Data,
            sequence: 42,
            payload: vec![1, 2, 3, 4],
        };
        let encoded = frame.encode();
        assert_eq!(encoded[0], FLAP_MAGIC_BYTE);

        let (channel, sequence, length) = FlapFrame::parse_header(&encoded[0..FLAP_HEADER_SIZE]).unwrap();
        assert_eq!(channel, FlapChannel::Data);
        assert_eq!(sequence, 42);
        assert_eq!(length as usize, frame.payload.len());
        assert_eq!(&encoded[FLAP_HEADER_SIZE..], &frame.payload[..]);
    }

    #[test]
    fn rejects_bad_magic_byte() {
        let bad_header = [0xff, 0x02, 0x00, 0x01, 0x00, 0x00];
        assert!(FlapFrame::parse_header(&bad_header).is_none());
    }

    #[test]
    fn rejects_wrong_length_header() {
        assert!(FlapFrame::parse_header(&[FLAP_MAGIC_BYTE, 0x02]).is_none());
    }
}
