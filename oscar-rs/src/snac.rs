//! A SNAC ("Simple Network Atomic Communication") is the actual command unit
//! inside a FLAP channel-2 data frame. Every login step, IM, buddy list
//! update, etc. is a SNAC identified by a (family, subtype) pair — e.g.
//! family 0x04 is "ICBM" (messaging), subtype 0x06 is "send IM".
//!
//! Wire format (big-endian), inside the FLAP payload:
//!   bytes 0-1: family    (u16)
//!   bytes 2-3: subtype   (u16)
//!   bytes 4-5: flags     (u16, usually 0)
//!   bytes 6-9: request_id (u32, client picks this, server echoes it back —
//!              useful for matching responses)
//!   bytes 10...: body (family/subtype specific — usually a run of TLVs)

use std::collections::HashMap;

pub const SNAC_HEADER_SIZE: usize = 10;

/// Formats bytes as space-separated hex, truncated so a stray huge body
/// doesn't flood the terminal — for `eprintln!` debugging against a real
/// server, where there's no Wireshark capture to fall back on.
pub(crate) fn hex_dump(data: &[u8]) -> String {
    const MAX: usize = 128;
    let shown = &data[..data.len().min(MAX)];
    let hex: Vec<String> = shown.iter().map(|b| format!("{b:02x}")).collect();
    if data.len() > MAX {
        format!("{} ... ({} more bytes)", hex.join(" "), data.len() - MAX)
    } else {
        hex.join(" ")
    }
}

/// The SNAC families implemented so far. There are more (chat rooms, file
/// transfer, directory search...) — add them here as features are built out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnacFamily {
    Generic = 0x0001,       // service-level: rate limits, host online/offline
    Locate = 0x0002,        // user profile + away message get/set
    BuddyPresence = 0x0003, // "Buddy" family — online/offline arrival notifications
    Messaging = 0x0004,     // ICBM — instant messages
    Feedbag = 0x0013,       // buddy list roster storage (add/remove/sync)
    Authorization = 0x0017, // BUCP — login/auth
}

impl SnacFamily {
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            0x0001 => Some(SnacFamily::Generic),
            0x0002 => Some(SnacFamily::Locate),
            0x0003 => Some(SnacFamily::BuddyPresence),
            0x0004 => Some(SnacFamily::Messaging),
            0x0013 => Some(SnacFamily::Feedbag),
            0x0017 => Some(SnacFamily::Authorization),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SnacHeader {
    pub family: u16,
    pub subtype: u16,
    pub flags: u16,
    pub request_id: u32,
}

impl SnacHeader {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SNAC_HEADER_SIZE);
        out.extend_from_slice(&self.family.to_be_bytes());
        out.extend_from_slice(&self.subtype.to_be_bytes());
        out.extend_from_slice(&self.flags.to_be_bytes());
        out.extend_from_slice(&self.request_id.to_be_bytes());
        out
    }

    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < SNAC_HEADER_SIZE {
            return None;
        }
        Some(SnacHeader {
            family: u16::from_be_bytes([data[0], data[1]]),
            subtype: u16::from_be_bytes([data[2], data[3]]),
            flags: u16::from_be_bytes([data[4], data[5]]),
            request_id: u32::from_be_bytes([data[6], data[7], data[8], data[9]]),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Snac {
    pub header: SnacHeader,
    pub body: Vec<u8>,
}

impl Snac {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.header.encode();
        out.extend_from_slice(&self.body);
        out
    }

    pub fn parse(data: &[u8]) -> Option<Self> {
        let header = SnacHeader::parse(data)?;
        let body = data[SNAC_HEADER_SIZE..].to_vec();
        Some(Snac { header, body })
    }
}

// MARK: - TLV (Type-Length-Value) encoding

/// Most SNAC bodies are built from TLVs rather than fixed structs — e.g. the
/// login request is a bag of TLVs (screen name, password hash, client
/// version...). Wire format: type (u16), length (u16), value (length bytes).
#[derive(Debug, Clone, PartialEq)]
pub struct Tlv {
    pub tlv_type: u16,
    pub value: Vec<u8>,
}

impl Tlv {
    pub fn new(tlv_type: u16, value: impl Into<Vec<u8>>) -> Self {
        Tlv { tlv_type, value: value.into() }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.value.len());
        out.extend_from_slice(&self.tlv_type.to_be_bytes());
        out.extend_from_slice(&(self.value.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.value);
        out
    }

    /// Parses a flat run of consecutive TLVs (how most SNAC bodies are
    /// structured). Malformed trailing bytes are silently dropped, matching
    /// the permissive parsing approach used throughout this scaffold.
    pub fn parse_all(data: &[u8]) -> HashMap<u16, Vec<u8>> {
        let (tlvs, _) = Self::parse_n(data, usize::MAX);
        tlvs
    }

    /// Parses at most `count` TLV entries starting at the front of `data`,
    /// returning them and how many bytes were consumed. Unlike `parse_all`
    /// (which just consumes everything it can), this is for the *bounded*
    /// TLV runs OSCAR uses in a few places — a `TLVBlock` (a TLV *count*
    /// prefix, not a byte length) rather than a `TLVRestBlock` — where a
    /// caller needs to know exactly where the bounded run ends so it can
    /// parse whatever follows it. See `UserInfo` and `FeedbagItem`.
    pub fn parse_n(data: &[u8], count: usize) -> (HashMap<u16, Vec<u8>>, usize) {
        let mut result = HashMap::new();
        let mut index = 0usize;
        for _ in 0..count {
            if index + 4 > data.len() {
                break;
            }
            let tlv_type = u16::from_be_bytes([data[index], data[index + 1]]);
            let length = u16::from_be_bytes([data[index + 2], data[index + 3]]) as usize;
            let value_start = index + 4;
            if value_start + length > data.len() {
                break;
            }
            result.insert(tlv_type, data[value_start..value_start + length].to_vec());
            index = value_start + length;
        }
        (result, index)
    }
}

// MARK: - UserInfo ("TLVUserInfo" in server-side terms)

/// The "user info" block OSCAR embeds in several SNACs — buddy
/// arrival/departure (family 0x03), incoming ICBM messages (family 0x04),
/// and Locate user-info replies (family 0x02). Confirmed against Open OSCAR
/// Server's own source (`wire.TLVUserInfo`): a length-prefixed screen name,
/// then a **raw** (non-TLV) warning level, then a TLV **count** (not a byte
/// length, unlike `Tlv::parse_all`'s bodies) followed by exactly that many
/// TLVs. Easy to mistake for "name followed by a plain TLV run" since the
/// screen-name framing looks the same as everywhere else — the warning
/// level and count fields in between are what `Tlv::parse_all` alone can't
/// account for.
#[derive(Debug, Clone)]
pub struct UserInfo {
    pub screen_name: String,
    pub warning_level: u16,
    pub tlvs: HashMap<u16, Vec<u8>>,
}

impl UserInfo {
    /// Parses one UserInfo block starting at the front of `data`, returning
    /// it and how many bytes it consumed — callers that have more data
    /// after it (e.g. an ICBM message's own TLVRestBlock) need that count
    /// to know where the rest starts.
    pub fn parse(data: &[u8]) -> Option<(UserInfo, usize)> {
        let &name_len = data.first()?;
        let name_len = name_len as usize;
        if data.len() < 1 + name_len + 4 {
            return None;
        }
        let screen_name = String::from_utf8_lossy(&data[1..1 + name_len]).to_string();
        let mut index = 1 + name_len;

        let warning_level = u16::from_be_bytes([data[index], data[index + 1]]);
        index += 2;

        let tlv_count = u16::from_be_bytes([data[index], data[index + 1]]) as usize;
        index += 2;

        let (tlvs, consumed) = Tlv::parse_n(&data[index..], tlv_count);
        index += consumed;

        Some((UserInfo { screen_name, warning_level, tlvs }, index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snac_header_round_trips() {
        let header = SnacHeader { family: 0x17, subtype: 0x06, flags: 0, request_id: 99 };
        let encoded = header.encode();
        let parsed = SnacHeader::parse(&encoded).unwrap();
        assert_eq!(parsed.family, 0x17);
        assert_eq!(parsed.subtype, 0x06);
        assert_eq!(parsed.request_id, 99);
    }

    #[test]
    fn snac_round_trips_with_body() {
        let snac = Snac {
            header: SnacHeader { family: 0x04, subtype: 0x06, flags: 0, request_id: 1 },
            body: vec![0xde, 0xad, 0xbe, 0xef],
        };
        let encoded = snac.encode();
        let parsed = Snac::parse(&encoded).unwrap();
        assert_eq!(parsed.body, vec![0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(parsed.header.family, 0x04);
    }

    #[test]
    fn tlv_round_trips_single() {
        let tlv = Tlv::new(0x01, b"MyScreenName".to_vec());
        let encoded = tlv.encode();
        let parsed = Tlv::parse_all(&encoded);
        assert_eq!(parsed.get(&0x01).unwrap(), b"MyScreenName");
    }

    #[test]
    fn tlv_parse_all_handles_multiple_tlvs() {
        let mut body = Vec::new();
        body.extend(Tlv::new(0x01, b"screenname".to_vec()).encode());
        body.extend(Tlv::new(0x02, vec![0x00, 0x01]).encode());

        let parsed = Tlv::parse_all(&body);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.get(&0x01).unwrap(), b"screenname");
        assert_eq!(parsed.get(&0x02).unwrap(), &vec![0x00, 0x01]);
    }

    #[test]
    fn tlv_parse_all_ignores_truncated_trailing_bytes() {
        let mut body = Tlv::new(0x01, b"ok".to_vec()).encode();
        body.extend_from_slice(&[0x00, 0x02]); // a dangling, incomplete TLV type+length
        let parsed = Tlv::parse_all(&body);
        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn user_info_parses_name_warning_and_tlvs() {
        let mut data = vec![3u8]; // name length
        data.extend_from_slice(b"Bob");
        data.extend_from_slice(&250u16.to_be_bytes()); // warning level (raw, not a TLV)
        data.extend_from_slice(&1u16.to_be_bytes()); // TLV count
        data.extend(Tlv::new(0x01, 0x0020u16.to_be_bytes().to_vec()).encode());
        data.extend_from_slice(&[0xAA, 0xBB]); // trailing bytes belonging to the caller, not this block

        let (info, consumed) = UserInfo::parse(&data).unwrap();
        assert_eq!(info.screen_name, "Bob");
        assert_eq!(info.warning_level, 250);
        assert_eq!(info.tlvs.get(&0x01).unwrap(), &0x0020u16.to_be_bytes().to_vec());
        assert_eq!(&data[consumed..], &[0xAA, 0xBB]);
    }

    #[test]
    fn user_info_handles_zero_tlvs() {
        let mut data = vec![3u8];
        data.extend_from_slice(b"Bob");
        data.extend_from_slice(&0u16.to_be_bytes());
        data.extend_from_slice(&0u16.to_be_bytes());

        let (info, consumed) = UserInfo::parse(&data).unwrap();
        assert_eq!(info.screen_name, "Bob");
        assert!(info.tlvs.is_empty());
        assert_eq!(consumed, data.len());
    }

    #[test]
    fn snac_family_round_trips_through_u16() {
        for family in [
            SnacFamily::Generic,
            SnacFamily::Locate,
            SnacFamily::BuddyPresence,
            SnacFamily::Messaging,
            SnacFamily::Feedbag,
            SnacFamily::Authorization,
        ] {
            assert_eq!(SnacFamily::from_u16(family.as_u16()), Some(family));
        }
    }
}
