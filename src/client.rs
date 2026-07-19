//! Orchestrates a full OSCAR login: connect to the auth server, exchange the
//! MD5-hashed password challenge, get handed off to the BOS (Basic OSCAR
//! Service) server, and land in a state where the caller has an open,
//! authenticated connection ready for messaging/buddy-list/etc.
//!
//! This targets Open OSCAR Server's default config. Against the real
//! (long-dead) AOL servers this same flow mostly applied too — the protocol
//! hasn't changed, only who's running it.

use crate::connection::FlapConnection;
use crate::flap::FlapChannel;
use crate::server_address::ServerAddress;
use crate::snac::{Snac, SnacFamily, SnacHeader, Tlv};

#[derive(Debug, thiserror::Error)]
pub enum OscarError {
    #[error("network error: {0}")]
    Io(#[from] std::io::Error),
    #[error("connection closed unexpectedly while {0}")]
    ConnectionClosed(&'static str),
    #[error("unexpected or malformed response: {0}")]
    UnexpectedResponse(&'static str),
    #[error("login rejected by server: {0}")]
    LoginFailed(String),
}

/// An authenticated session, holding the live BOS connection. Messaging,
/// buddy list, and away-status calls (not yet ported from the Swift
/// scaffold) will be methods on this once they land.
pub struct OscarSession {
    pub bos_connection: FlapConnection,
    pub screen_name: String,
}

/// The *only* password hashing OSCAR uses: a chained MD5 combining the
/// server's challenge, the MD5 of the password itself, and a fixed client
/// identifier string. This exact scheme (not just "MD5 the password") is
/// what libpurple's OSCAR module implements and is the de facto reference,
/// there being no official spec.
fn roast_password(auth_key: &[u8], password: &str) -> [u8; 16] {
    let password_digest = md5::compute(password.as_bytes()).0;
    let mut combined = Vec::with_capacity(auth_key.len() + 16 + 27);
    combined.extend_from_slice(auth_key);
    combined.extend_from_slice(&password_digest);
    combined.extend_from_slice(b"AOL Instant Messenger (SM)");
    md5::compute(&combined).0
}

/// Simple monotonic counter for SNAC request IDs. The client picks these;
/// the server echoes them back, useful for matching responses to requests
/// once there's more than one in flight at a time (not needed yet for the
/// strictly sequential login flow, but the connection/messaging layer will
/// want it).
struct RequestIdCounter(u32);
impl RequestIdCounter {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(1);
        self.0
    }
}

pub async fn login(server: &ServerAddress, screen_name: &str, password: &str) -> Result<OscarSession, OscarError> {
    let mut ids = RequestIdCounter(0);
    let mut auth = FlapConnection::connect(server).await?;

    // Channel 1 "hello": 4-byte FLAP protocol version, always 1.
    auth.send(FlapChannel::NewConnection, 1u32.to_be_bytes().to_vec()).await?;

    // Request an auth key by sending our screen name.
    // SNAC family 0x17 (BUCP), subtype 0x06 = "request login challenge".
    let name_tlv = Tlv::new(0x01, screen_name.as_bytes().to_vec());
    let header = SnacHeader {
        family: SnacFamily::Authorization.as_u16(),
        subtype: 0x06,
        flags: 0,
        request_id: ids.next(),
    };
    auth.send_snac(&Snac { header, body: name_tlv.encode() }).await?;

    // Wait for the auth key (challenge) response, ignoring any unrelated
    // traffic in between (real servers can interleave other frames).
    let auth_key = loop {
        let frame = auth
            .read_frame()
            .await?
            .ok_or(OscarError::ConnectionClosed("waiting for auth key"))?;
        if frame.channel != FlapChannel::Data {
            continue;
        }
        let Some(snac) = Snac::parse(&frame.payload) else { continue };
        if snac.header.family == SnacFamily::Authorization.as_u16() && snac.header.subtype == 0x07 {
            let tlvs = Tlv::parse_all(&snac.body);
            break tlvs
                .get(&0x01)
                .cloned()
                .ok_or(OscarError::UnexpectedResponse("auth key TLV (0x01) missing from challenge reply"))?;
        }
    };

    // Roasted MD5: MD5( authKey + MD5(password) + "AOL Instant Messenger (SM)" ).
    let hash = roast_password(&auth_key, password);

    let mut body = Vec::new();
    body.extend(Tlv::new(0x01, screen_name.as_bytes().to_vec()).encode());
    body.extend(Tlv::new(0x25, hash.to_vec()).encode());
    body.extend(Tlv::new(0x03, b"oscar-rs/0.1".to_vec()).encode()); // client ID string

    let header = SnacHeader {
        family: SnacFamily::Authorization.as_u16(),
        subtype: 0x02,
        flags: 0,
        request_id: ids.next(),
    };
    auth.send_snac(&Snac { header, body }).await?;

    // Wait for the login response: either an error (TLV 0x08) or success
    // with a BOS server address (TLV 0x05) + session cookie (TLV 0x06).
    let (bos_address_str, cookie) = loop {
        let frame = auth
            .read_frame()
            .await?
            .ok_or(OscarError::ConnectionClosed("waiting for login response"))?;
        if frame.channel != FlapChannel::Data {
            continue;
        }
        let Some(snac) = Snac::parse(&frame.payload) else { continue };
        if snac.header.family == SnacFamily::Authorization.as_u16() && snac.header.subtype == 0x03 {
            let tlvs = Tlv::parse_all(&snac.body);

            if let Some(error_bytes) = tlvs.get(&0x08) {
                let code = if error_bytes.len() >= 2 {
                    u16::from_be_bytes([error_bytes[0], error_bytes[1]])
                } else {
                    0
                };
                return Err(OscarError::LoginFailed(format!("BUCP error code {code}")));
            }

            let bos_bytes = tlvs
                .get(&0x05)
                .ok_or(OscarError::UnexpectedResponse("missing BOS server address (TLV 0x05)"))?;
            let cookie = tlvs
                .get(&0x06)
                .ok_or(OscarError::UnexpectedResponse("missing auth cookie (TLV 0x06)"))?
                .clone();
            break (String::from_utf8_lossy(bos_bytes).to_string(), cookie);
        }
    };

    // Done with the auth connection — the rest of the session happens on BOS.
    drop(auth);

    let bos_address = ServerAddress::parse(&bos_address_str)
        .map_err(|_| OscarError::UnexpectedResponse("server sent an unparseable BOS address"))?;
    let mut bos = FlapConnection::connect(&bos_address).await?;

    // Channel 1 hello again, but this time carrying the auth cookie as a TLV
    // so the BOS server knows which just-authenticated session this is.
    let mut hello_payload = 1u32.to_be_bytes().to_vec();
    hello_payload.extend(Tlv::new(0x06, cookie).encode());
    bos.send(FlapChannel::NewConnection, hello_payload).await?;

    // Wait for "host online" (family Generic, subtype 0x03) — the signal
    // that the BOS server is ready and login has fully succeeded.
    loop {
        let frame = bos
            .read_frame()
            .await?
            .ok_or(OscarError::ConnectionClosed("waiting for host online"))?;
        if frame.channel != FlapChannel::Data {
            continue;
        }
        let Some(snac) = Snac::parse(&frame.payload) else { continue };
        if snac.header.family == SnacFamily::Generic.as_u16() && snac.header.subtype == 0x03 {
            break;
        }
    }

    Ok(OscarSession { bos_connection: bos, screen_name: screen_name.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roast_password_is_deterministic() {
        let key = b"some-challenge-bytes";
        let a = roast_password(key, "hunter2");
        let b = roast_password(key, "hunter2");
        assert_eq!(a, b);
    }

    #[test]
    fn roast_password_changes_with_password() {
        let key = b"some-challenge-bytes";
        let a = roast_password(key, "hunter2");
        let b = roast_password(key, "different-password");
        assert_ne!(a, b);
    }

    #[test]
    fn roast_password_changes_with_auth_key() {
        let a = roast_password(b"challenge-one", "hunter2");
        let b = roast_password(b"challenge-two", "hunter2");
        assert_ne!(a, b, "same password, different server challenge, must produce different hashes");
    }

    #[test]
    fn roast_password_matches_hand_computed_reference() {
        // Manually replicates the chained-MD5 scheme to guard against a
        // future refactor accidentally changing the byte order or fixed
        // string — this is the closest we can get to a "known answer test"
        // without a real server capture to compare against.
        let key = b"abc123";
        let password_digest = md5::compute(b"hunter2").0;
        let mut combined = Vec::new();
        combined.extend_from_slice(key);
        combined.extend_from_slice(&password_digest);
        combined.extend_from_slice(b"AOL Instant Messenger (SM)");
        let expected = md5::compute(&combined).0;

        assert_eq!(roast_password(key, "hunter2"), expected);
    }

    #[test]
    fn request_id_counter_increments_and_wraps() {
        let mut ids = RequestIdCounter(u32::MAX - 1);
        assert_eq!(ids.next(), u32::MAX);
        assert_eq!(ids.next(), 0); // wraps rather than panicking
    }
}
