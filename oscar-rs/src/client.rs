//! Orchestrates a full OSCAR login: connect to the auth server, exchange the
//! MD5-hashed password challenge, get handed off to the BOS (Basic OSCAR
//! Service) server, and land in a state where the caller has an open,
//! authenticated connection ready for messaging/buddy-list/etc.
//!
//! This targets Open OSCAR Server's default config. Against the real
//! (long-dead) AOL servers this same flow mostly applied too — the protocol
//! hasn't changed, only who's running it.

use std::collections::HashMap;

use crate::connection::{FlapConnection, FlapReader, FlapWriter};
use crate::feedbag::{Buddy, FeedbagItem};
use crate::flap::{FlapChannel, FlapFrame};
use crate::messaging::IncomingIm;
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

/// An authenticated session, holding the live BOS connection plus the state
/// ported from the Swift scaffold: the synced buddy list (`feedbag.rs`),
/// your own away message and buddies' (`locate.rs`), and received instant
/// messages (`messaging.rs`). Call `handle_next_frame` in a loop to keep
/// this state current as the server pushes updates.
pub struct OscarSession {
    pub bos_connection: FlapWriter,
    /// The read half of the BOS connection. Taken out via `split_reader()`
    /// by callers (like the Tauri layer) that want to run the read loop on
    /// a dedicated task instead of calling `handle_next_frame` directly —
    /// see that method's doc comment for why this matters.
    bos_reader: Option<FlapReader>,
    pub screen_name: String,

    /// Your synced buddy list, reconciled from feedbag + live presence
    /// updates. See `feedbag.rs` for how this gets populated.
    pub buddies: Vec<Buddy>,
    /// Raw feedbag items as last synced from the server — buddies, groups,
    /// and meta-items. `buddies` above is the UI-friendly projection of
    /// this; this raw form is kept around because add/remove operations
    /// need to look up existing group IDs and item IDs.
    pub feedbag_items: Vec<FeedbagItem>,
    /// Your own current away message. `None` means available. See
    /// `locate.rs` — setting this via `set_away_message` is the actual
    /// mechanism that makes you appear away to buddies; there's no separate
    /// away/available toggle in OSCAR.
    pub away_message: Option<String>,
    /// Instant messages received so far, in arrival order.
    pub incoming_messages: Vec<IncomingIm>,

    ids: RequestIdCounter,
    feedbag_item_id_counter: u16,
    /// Screen names of buddies we've sent an ICBM warning to, keyed by the
    /// request_id of that warning SNAC, so the (screen-name-less) reply can
    /// be attributed back to the right buddy. See `messaging.rs::send_warning`.
    pub(crate) pending_warnings: HashMap<u32, String>,
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
/// once there's more than one in flight at a time — used throughout the
/// feedbag/locate/messaging methods on `OscarSession`.
pub(crate) struct RequestIdCounter(u32);
impl RequestIdCounter {
    pub(crate) fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(1);
        self.0
    }
}

impl OscarSession {
    pub(crate) fn next_request_id(&mut self) -> u32 {
        self.ids.next()
    }

    /// Feedbag item IDs are scoped per-account, chosen by the client, and
    /// must not collide with existing items. A monotonic counter seeded
    /// above any ID we've seen from the server is good enough for a v0.1 —
    /// a real app should persist the high-water mark rather than restart
    /// from 1 each launch.
    pub(crate) fn next_feedbag_item_id(&mut self) -> u16 {
        let existing_max = self.feedbag_items.iter().map(|i| i.item_id).max().unwrap_or(0);
        self.feedbag_item_id_counter = self.feedbag_item_id_counter.max(existing_max).wrapping_add(1);
        self.feedbag_item_id_counter
    }

    /// Takes the read half of the BOS connection out of the session so a
    /// caller can run it on its own task — e.g. the Tauri layer's dedicated
    /// reader task, which forwards parsed frames over a channel to an actor
    /// that owns the rest of the session. Panics if called twice on the
    /// same session (there's only one read half to give out).
    pub fn split_reader(&mut self) -> FlapReader {
        self.bos_reader.take().expect("split_reader() called twice on the same OscarSession")
    }

    /// Reads one FLAP frame from the BOS connection and, if it carries a
    /// SNAC this client understands, dispatches it to the matching
    /// handler — updating `buddies`, `incoming_messages`, `away_message`,
    /// etc. in place. Call this in a loop once logged in to keep session
    /// state current. If you've called `split_reader()` (e.g. to run the
    /// read loop on a separate task), read frames from that `FlapReader`
    /// instead and pass them to `dispatch_frame` directly.
    pub async fn handle_next_frame(&mut self) -> Result<(), OscarError> {
        let reader = self.bos_reader.as_mut().expect("bos_reader missing — was split_reader() already called?");
        let frame = reader.read_frame().await?.ok_or(OscarError::ConnectionClosed("bos session"))?;
        self.dispatch_frame(frame).await
    }

    /// Parses and dispatches a single FLAP frame already read off the BOS
    /// connection — the shared logic behind `handle_next_frame`, split out
    /// so a caller running its own read loop (via `split_reader`) can feed
    /// frames in without going through this session's own reader half.
    pub async fn dispatch_frame(&mut self, frame: FlapFrame) -> Result<(), OscarError> {
        if frame.channel != FlapChannel::Data {
            return Ok(());
        }
        let Some(snac) = Snac::parse(&frame.payload) else { return Ok(()) };

        match SnacFamily::from_u16(snac.header.family) {
            Some(SnacFamily::Messaging) => match snac.header.subtype {
                crate::messaging::INCOMING_IM => {
                    if let Some(im) = crate::messaging::parse_incoming_im(&snac.body) {
                        self.incoming_messages.push(im);
                    }
                }
                crate::messaging::WARNING_REPLY => self.handle_warning_reply(&snac),
                _ => {}
            },
            Some(SnacFamily::Feedbag) => self.handle_feedbag_frame(&snac).await?,
            Some(SnacFamily::BuddyPresence) => self.handle_presence_frame(&snac),
            Some(SnacFamily::Locate) => self.handle_locate_frame(&snac),
            _ => {}
        }
        Ok(())
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

    let (bos_reader, bos_writer) = bos.into_split();
    let mut session = OscarSession {
        bos_connection: bos_writer,
        bos_reader: Some(bos_reader),
        screen_name: screen_name.to_string(),
        buddies: Vec::new(),
        feedbag_items: Vec::new(),
        away_message: None,
        incoming_messages: Vec::new(),
        ids: RequestIdCounter(0),
        feedbag_item_id_counter: 1,
        pending_warnings: HashMap::new(),
    };

    // Roster is foundational session state — fetch it as soon as we're
    // online, same as real clients do before anything else becomes
    // meaningful. The reply arrives async; consume it via `handle_next_frame`.
    session.request_buddy_list().await?;

    Ok(session)
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
