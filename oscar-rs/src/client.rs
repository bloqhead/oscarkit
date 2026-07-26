//! Orchestrates a full OSCAR login: connect to the auth server, exchange the
//! MD5-hashed password challenge, get handed off to the BOS (Basic OSCAR
//! Service) server, and land in a state where the caller has an open,
//! authenticated connection ready for messaging/buddy-list/etc.
//!
//! This targets Open OSCAR Server's default config. Against the real
//! (long-dead) AOL servers this same flow mostly applied too — the protocol
//! hasn't changed, only who's running it.

use std::collections::HashMap;

use crate::chat::ChatRoomSession;
use crate::chat_nav::{ChatNavSession, ChatRoomHandle};
use crate::connection::{FlapConnection, FlapReader, FlapWriter};
use crate::feedbag::{Buddy, FeedbagItem};
use crate::flap::{FlapChannel, FlapFrame};
use crate::messaging::{ChatInvite, IncomingIm};
use crate::server_address::ServerAddress;
use crate::snac::{hex_dump, Snac, SnacFamily, SnacHeader, Tlv};

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
    /// Chat-room invite proposals received so far (ICBM channel 2), in
    /// arrival order. See `messaging.rs` for why these arrive via the same
    /// ICBM incoming-message SNAC as regular IMs, distinguished only by a
    /// body-level channel field.
    pub incoming_chat_invites: Vec<ChatInvite>,

    ids: RequestIdCounter,
    feedbag_item_id_counter: u16,
    /// Screen names of buddies we've sent an ICBM warning to, keyed by the
    /// request_id of that warning SNAC, so the (screen-name-less) reply can
    /// be attributed back to the right buddy. See `messaging.rs::send_warning`.
    pub(crate) pending_warnings: HashMap<u32, String>,
    /// Results of `OServiceServiceRequest`s (redirect to ChatNav or a
    /// specific Chat room) whose `OServiceServiceResponse` has already been
    /// seen by `dispatch_frame`, keyed by request_id, waiting to be claimed
    /// by whoever sent the request. See `request_service_redirect`'s doc
    /// comment for why this is a poll-able cache rather than a oneshot: in
    /// real (Tauri actor) usage, nothing else can concurrently call
    /// `dispatch_frame` while a caller is `.await`ing inside
    /// `request_service_redirect` — the actor's own frame-processing loop
    /// *is* what's suspended by that call. `request_service_redirect` has
    /// to pull and dispatch further frames itself (via the `FrameSource` it
    /// takes as a parameter) rather than relying on a concurrent waker.
    pending_service_redirect_replies: HashMap<u32, (ServerAddress, Vec<u8>)>,
}

/// Where `request_service_redirect` (and the room-create/join flow built on
/// it) gets its next frame from, while a reply to a request it sent is still
/// outstanding. Generic rather than a concrete type because the answer is
/// different depending on whether `OscarSession::split_reader()` has been
/// called yet: before that, a `FlapReader` itself works directly (tests, or
/// any non-actor usage — reads straight off the socket). After that (real
/// Tauri actor usage), the actual `FlapReader` is owned by a dedicated
/// reader task forwarding frames over a channel, so the Tauri layer
/// implements this trait over that channel's receiver instead — see
/// `src-tauri/src/chat_actor.rs`.
pub trait FrameSource {
    async fn next_frame(&mut self) -> Result<FlapFrame, OscarError>;
}

impl FrameSource for FlapReader {
    async fn next_frame(&mut self) -> Result<FlapFrame, OscarError> {
        self.read_frame().await?.ok_or(OscarError::ConnectionClosed("bos session"))
    }
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
    pub(crate) fn new() -> Self {
        RequestIdCounter(0)
    }

    pub(crate) fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(1);
        self.0
    }
}

/// OSCAR screen names are canonically case- and whitespace-insensitive —
/// confirmed the hard way: a real presence arrival named a buddy
/// `"Catmints"` while that same buddy's feedbag-list entry (what actually
/// populated `OscarSession::buddies`) was `"catmints"`. A plain `==` on
/// screen names — used throughout `feedbag.rs`/`locate.rs` to match an
/// incoming SNAC's screen name against the local buddy list — silently
/// fails to find the buddy whenever the two sides disagree on casing,
/// which is routine, not an edge case: presence/warning/locate replies and
/// feedbag-list entries have no guarantee of using the same display form.
pub(crate) fn screen_names_match(a: &str, b: &str) -> bool {
    fn normalize(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect()
    }
    normalize(a) == normalize(b)
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
        let Some(snac) = Snac::parse(&frame.payload) else {
            eprintln!("[oscar-rs] dropped an unparseable FLAP data frame ({} bytes)", frame.payload.len());
            return Ok(());
        };

        eprintln!(
            "[oscar-rs] <- family=0x{:04x} subtype=0x{:02x} body={} bytes: {}",
            snac.header.family,
            snac.header.subtype,
            snac.body.len(),
            hex_dump(&snac.body)
        );

        // Family 0x01 (Generic) subtype 0x01 is the server's catch-all
        // "here's why I'm about to close/refuse this" error SNAC — very
        // relevant when tracking down an unexpected disconnect.
        if snac.header.family == SnacFamily::Generic.as_u16() && snac.header.subtype == 0x01 {
            eprintln!("[oscar-rs] *** server sent a Generic error SNAC: {}", hex_dump(&snac.body));
        }

        match SnacFamily::from_u16(snac.header.family) {
            // OServiceServiceResponse — the reply to a mid-session redirect
            // request (ChatNav or a specific Chat room). Chat/ChatNav frames
            // themselves never arrive on BOS (they arrive on their own
            // dedicated connections once redirected), so this is the only
            // Generic-family subtype this dispatch needs to special-case
            // beyond the error-SNAC logging above.
            Some(SnacFamily::Generic) if snac.header.subtype == 0x05 => {
                let tlvs = Tlv::parse_all(&snac.body);
                if let Ok(result) = parse_redirect_address_and_cookie(&tlvs) {
                    self.pending_service_redirect_replies.insert(snac.header.request_id, result);
                }
            }
            Some(SnacFamily::Messaging) => match snac.header.subtype {
                crate::messaging::INCOMING_IM => match crate::messaging::icbm_channel(&snac.body) {
                    Some(2) => {
                        if let Some(invite) = crate::messaging::parse_chat_invite(&snac.body) {
                            self.incoming_chat_invites.push(invite);
                        }
                    }
                    _ => {
                        if let Some(im) = crate::messaging::parse_incoming_im(&snac.body) {
                            self.incoming_messages.push(im);
                        }
                    }
                },
                crate::messaging::WARNING_REPLY => self.handle_warning_reply(&snac),
                _ => {}
            },
            Some(SnacFamily::Feedbag) => self.handle_feedbag_frame(&snac).await?,
            Some(SnacFamily::BuddyPresence) => self.handle_presence_frame(&snac),
            Some(SnacFamily::Locate) => self.handle_locate_frame(&snac),
            other => eprintln!("[oscar-rs] no handler for family {other:?} (0x{:04x}) — ignored", snac.header.family),
        }
        Ok(())
    }

    /// Sends SNAC(0x01,0x04) `OServiceServiceRequest` on the live BOS
    /// connection requesting a redirect to `food_group` (ChatNav = 0x000D,
    /// or a specific Chat room = 0x000E), with `extra_tlvs` carrying
    /// whatever the target needs to identify the request (a Chat redirect
    /// needs the room's exchange/cookie/instance as TLV 0x01, a
    /// `SNAC_0x01_0x04_TLVRoomInfo`-shaped triple — see `chat_nav.rs`).
    /// Awaits the matching `OServiceServiceResponse` by pulling further BOS
    /// frames from `frames` and dispatching each through `self` (which
    /// populates `pending_service_redirect_replies`) until the one for this
    /// request shows up. `frames` is injected rather than read directly off
    /// `self.bos_reader` because, inside the real Tauri actor, the reader is
    /// already split out and owned by a separate task — this method has to
    /// be able to pull frames from whatever the caller has on hand (a
    /// `FlapReader` directly in tests, or a thin wrapper around the actor's
    /// `mpsc::Receiver<FrameEvent>` in production) instead of assuming
    /// exclusive ownership of a socket it doesn't have.
    async fn request_service_redirect<S: FrameSource>(
        &mut self,
        food_group: u16,
        extra_tlvs: Vec<Tlv>,
        frames: &mut S,
    ) -> Result<(ServerAddress, Vec<u8>), OscarError> {
        let mut body = food_group.to_be_bytes().to_vec();
        for tlv in extra_tlvs {
            body.extend(tlv.encode());
        }
        let request_id = self.next_request_id();

        let header = SnacHeader { family: SnacFamily::Generic.as_u16(), subtype: 0x04, flags: 0, request_id };
        self.bos_connection.send_snac(&Snac { header, body }).await?;

        loop {
            if let Some(result) = self.pending_service_redirect_replies.remove(&request_id) {
                return Ok(result);
            }
            let frame = frames.next_frame().await?;
            self.dispatch_frame(frame).await?;
        }
    }

    /// Full create flow: BOS -> ChatNav redirect, connect, create the room,
    /// drop the ChatNav connection (it's ephemeral — nothing about a room
    /// needs it again after creation), then join the newly created room
    /// exactly like `join_room` would.
    pub async fn create_and_join_room<S: FrameSource>(&mut self, room_name: &str, frames: &mut S) -> Result<ChatRoomSession, OscarError> {
        let (address, cookie) = self.request_service_redirect(SnacFamily::ChatNav.as_u16(), Vec::new(), frames).await?;
        let mut chat_nav = ChatNavSession::connect(&address, cookie).await?;
        let handle = chat_nav.create_room(room_name).await?;
        drop(chat_nav);
        self.redirect_and_join(handle, frames).await
    }

    /// Joins a room identified by a handle obtained elsewhere — e.g. from an
    /// accepted invite (`incoming_chat_invites`), which carries the
    /// exchange/cookie/instance directly without ever needing to talk to
    /// ChatNav at all.
    pub async fn join_room<S: FrameSource>(&mut self, handle: &ChatRoomHandle, frames: &mut S) -> Result<ChatRoomSession, OscarError> {
        self.redirect_and_join(handle.clone(), frames).await
    }

    async fn redirect_and_join<S: FrameSource>(&mut self, handle: ChatRoomHandle, frames: &mut S) -> Result<ChatRoomSession, OscarError> {
        let mut room_selector = handle.exchange.to_be_bytes().to_vec();
        let cookie_bytes = handle.room_cookie.as_bytes();
        room_selector.push(cookie_bytes.len() as u8);
        room_selector.extend_from_slice(cookie_bytes);
        room_selector.extend_from_slice(&handle.instance.to_be_bytes());

        let (address, cookie) = self
            .request_service_redirect(SnacFamily::Chat.as_u16(), vec![Tlv::new(0x01, room_selector)], frames)
            .await?;
        let mut connection = connect_redirect(&address, cookie).await?;

        // Announce only the Chat family here — unlike BOS's ClientOnline
        // (which lists every family this client supports and is what makes
        // the server consider sign-on "complete"), a chat room's bootstrap
        // is different: the server proactively pushes the occupant list and
        // room metadata once it sees us online, no broader announcement
        // needed or expected.
        let mut ids = RequestIdCounter::new();
        let client_online_body = {
            let mut body = SnacFamily::Chat.as_u16().to_be_bytes().to_vec();
            body.extend_from_slice(&1u16.to_be_bytes()); // version
            body.extend_from_slice(&0u16.to_be_bytes()); // tool ID
            body.extend_from_slice(&0u16.to_be_bytes()); // tool version
            body
        };
        let header = SnacHeader { family: SnacFamily::Generic.as_u16(), subtype: 0x02, flags: 0, request_id: ids.next() };
        connection.send_snac(&Snac { header, body: client_online_body }).await?;

        // Mandated join sequence (order matters — see chat.rs/research doc):
        // full occupant list (joiner only), then room metadata (joiner
        // only). The third message (self-only ChatUsersJoined broadcast to
        // everyone *else*) isn't ours to wait for.
        let mut occupants = Vec::new();
        let mut room_handle = handle;
        loop {
            let frame = connection.read_frame().await?.ok_or(OscarError::ConnectionClosed("waiting for chat join sequence"))?;
            if frame.channel != FlapChannel::Data {
                continue;
            }
            let Some(snac) = Snac::parse(&frame.payload) else { continue };
            if SnacFamily::from_u16(snac.header.family) != Some(SnacFamily::Chat) {
                continue;
            }
            match snac.header.subtype {
                0x03 => occupants = crate::chat::parse_user_info_list(&snac.body),
                0x02 => {
                    if let Some(updated) = crate::chat_nav::parse_chat_room_info_update(&snac.body) {
                        room_handle = updated;
                    }
                    break;
                }
                _ => {}
            }
        }

        let (reader, writer) = connection.into_split();
        let mut session = ChatRoomSession::new(writer, reader, room_handle, self.screen_name.clone());
        session.occupants = occupants;
        Ok(session)
    }
}

/// Connects to any OSCAR service-redirect target the same way — ChatNav, a
/// specific joined Chat room, or BOS itself during login: a fresh FLAP
/// connection, a channel-1 hello carrying the handoff cookie as TLV 0x06,
/// then block until the server's "host online" signal (Generic family,
/// subtype 0x03). Shared by `login()`'s BOS hop and
/// `OscarSession::redirect_and_join`'s Chat hop (`chat_nav.rs`'s
/// `ChatNavSession::connect` also uses this shape directly).
pub(crate) async fn connect_redirect(address: &ServerAddress, cookie: Vec<u8>) -> Result<FlapConnection, OscarError> {
    let mut connection = FlapConnection::connect(address).await?;

    let mut hello_payload = 1u32.to_be_bytes().to_vec();
    hello_payload.extend(Tlv::new(0x06, cookie).encode());
    connection.send(FlapChannel::NewConnection, hello_payload).await?;

    loop {
        let frame = connection
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

    Ok(connection)
}

/// TLV 0x05 (redirect target address string) + TLV 0x06 (opaque handoff
/// cookie) — the pair every `OServiceServiceResponse` carries, including
/// the BUCP login response itself (which is structurally the same redirect
/// mechanism, just arriving over the auth-specific SNAC family rather than
/// Generic).
pub(crate) fn parse_redirect_address_and_cookie(tlvs: &HashMap<u16, Vec<u8>>) -> Result<(ServerAddress, Vec<u8>), OscarError> {
    let address_bytes = tlvs.get(&0x05).ok_or(OscarError::UnexpectedResponse("missing redirect address (TLV 0x05)"))?;
    let cookie = tlvs.get(&0x06).ok_or(OscarError::UnexpectedResponse("missing redirect cookie (TLV 0x06)"))?.clone();
    let address_str = String::from_utf8_lossy(address_bytes).to_string();
    let address = ServerAddress::parse(&address_str)
        .map_err(|_| OscarError::UnexpectedResponse("server sent an unparseable redirect address"))?;
    Ok((address, cookie))
}

pub async fn login(server: &ServerAddress, screen_name: &str, password: &str) -> Result<OscarSession, OscarError> {
    let mut ids = RequestIdCounter::new();
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
            // Confirmed against Open OSCAR Server's own source (wire.SNAC_0x17_0x07_BUCPChallengeResponse):
            // unlike the login request/response, this body is NOT a TLV block — it's a
            // plain `oscar:"len_prefix=uint16"` string: 2-byte big-endian length, then
            // that many bytes of auth key, nothing else.
            let body = &snac.body;
            if body.len() < 2 {
                return Err(OscarError::UnexpectedResponse("challenge reply shorter than its length prefix"));
            }
            let key_len = u16::from_be_bytes([body[0], body[1]]) as usize;
            if body.len() < 2 + key_len {
                return Err(OscarError::UnexpectedResponse("challenge reply truncated before end of auth key"));
            }
            break body[2..2 + key_len].to_vec();
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
    // with a BOS server address (TLV 0x05) + session cookie (TLV 0x06) —
    // the same two TLVs every OServiceServiceResponse redirect carries, see
    // `parse_redirect_address_and_cookie`.
    let (bos_address, cookie) = loop {
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

            break parse_redirect_address_and_cookie(&tlvs)?;
        }
    };

    // Done with the auth connection — the rest of the session happens on BOS.
    drop(auth);

    // Fresh FLAP connection, channel-1 hello carrying the auth cookie as a
    // TLV so the BOS server knows which just-authenticated session this is,
    // then block until "host online" — the same redirect-handoff shape
    // ChatNav/Chat connections use later, factored out since it's identical.
    let mut bos = connect_redirect(&bos_address, cookie).await?;

    // Announce "client online" (Generic family, subtype 0x02) — a list of
    // every SNAC family/version this client supports. Confirmed against
    // Open OSCAR Server's foodgroup/oservice.go: the server doesn't
    // consider sign-on complete until this arrives (it's what calls
    // SetSignonComplete() and starts broadcasting presence to buddies).
    // Skipping it leaves the TCP session alive but invisible — buddies
    // never see you online, and messages to/from you fail server-side with
    // "not logged on" even though you're genuinely connected. No count
    // prefix: just that many 8-byte (family, version, tool ID, tool
    // version) entries back to back, filling the rest of the SNAC body.
    let mut client_online_body = Vec::new();
    for family in [SnacFamily::Generic, SnacFamily::Locate, SnacFamily::BuddyPresence, SnacFamily::Messaging, SnacFamily::Feedbag] {
        client_online_body.extend_from_slice(&family.as_u16().to_be_bytes());
        client_online_body.extend_from_slice(&1u16.to_be_bytes()); // version
        client_online_body.extend_from_slice(&0u16.to_be_bytes()); // tool ID
        client_online_body.extend_from_slice(&0u16.to_be_bytes()); // tool version
    }
    let header = SnacHeader { family: SnacFamily::Generic.as_u16(), subtype: 0x02, flags: 0, request_id: ids.next() };
    bos.send_snac(&Snac { header, body: client_online_body }).await?;

    let (bos_reader, bos_writer) = bos.into_split();
    let mut session = OscarSession {
        bos_connection: bos_writer,
        bos_reader: Some(bos_reader),
        screen_name: screen_name.to_string(),
        buddies: Vec::new(),
        feedbag_items: Vec::new(),
        away_message: None,
        incoming_messages: Vec::new(),
        incoming_chat_invites: Vec::new(),
        ids: RequestIdCounter::new(),
        feedbag_item_id_counter: 1,
        pending_warnings: HashMap::new(),
        pending_service_redirect_replies: HashMap::new(),
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
    fn screen_names_match_ignores_case_and_whitespace() {
        assert!(screen_names_match("Catmints", "catmints"));
        assert!(screen_names_match("Screen Name", "screenname"));
        assert!(screen_names_match("SAME", "SAME"));
        assert!(!screen_names_match("Catmints", "Lyrix18"));
    }

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
