# Chat rooms — research (shelved)

Investigated as a possible feature. The server side already supports it —
this is purely a client-side gap — but the client-side lift is much bigger
than it looks, comparable in size to everything built so far combined (login
through messaging). Shelved for now; this doc preserves the research so it
doesn't need re-deriving later.

Verified against the real, open-source `mk6i/open-oscar-server` Go
implementation (`wire/snacs.go`, `foodgroup/chat_nav.go`, `foodgroup/chat.go`
and their test files) — not assumed from general AIM protocol history.

## The core fact that makes this bigger than "add a protocol module"

Chat rooms reuse the exact same generic OService connection-redirect
mechanism BOS itself uses on login (SNAC 0x01,0x04 `OServiceServiceRequest`
→ SNAC 0x01,0x05 `OServiceServiceResponse` carrying a new host/port + auth
cookie to reconnect with). It's reused twice more:

1. Once for a **ChatNav connection** — ephemeral, just for room lookup/
   creation.
2. Once **per joined room** for a dedicated **Chat connection** — persistent
   for as long as you're in that room.

This client's current architecture has exactly one live connection (BOS) per
session. Supporting chat rooms means N *additional* concurrent connections,
each with its own FLAP read loop, one per room the user has joined — a
genuinely new connection-management paradigm, not an extension of the
existing one.

## Protocol flow

**1. Get a ChatNav connection (from BOS):**
- Client → BOS: SNAC(0x01,0x04) `OServiceServiceRequest{FoodGroup: 0x000D}`
- Server → BOS: SNAC(0x01,0x05) `OServiceServiceResponse{TLVRestBlock}` — TLV
  0x0D food group, TLV 0x05 host:port, TLV 0x06 opaque auth cookie, TLV 0x8E
  SSL state.
- Client opens a **new TCP connection** to that host, FLAP handshake + auth
  using the cookie (same shape as the login→BOS handoff already implemented).

**2a. Discover an existing room (on ChatNav):**
- Client → ChatNav: SNAC(0x0D,0x04) `ChatNavRequestRoomInfo{Exchange u16;
  Cookie string(len_prefix=u8); InstanceNumber u16; DetailLevel u8}`
- Server → ChatNav: SNAC(0x0D,0x09) `ChatNavNavInfo{TLVRestBlock}` — TLV 0x04
  wraps a `ChatRoomInfoUpdate` (exchange/cookie/instance + a TLV block of
  room metadata: 0xD3 name, 0xD2 max occupancy, 0xD1 max msg len, 0xCA create
  time, 0xC9 flags).

**2b. Create a room (on ChatNav):**
- Client → ChatNav: SNAC(0x0D,0x08) `ChatNavCreateRoom` — reuses the *Chat*
  family's `ChatRoomInfoUpdate` struct, with `Cookie: "create"`.
- Server responds with the same `ChatNavNavInfo`, now carrying the real
  cookie/instance number.
- Two exchange types: `PrivateExchange` (create-on-demand) vs
  `PublicExchange` (must already exist, else `ErrorCodeNoMatch`).
- `ChatNavRequestChatRights` (0x0D,0x02) returns exchange config (name-length
  limits, class permissions) via the same `ChatNavNavInfo` reply shape, keyed
  by `ChatNavTLVExchangeInfo` (0x03) TLVs.

**3. Redirect to the room's dedicated Chat connection (back on BOS, not
ChatNav):**
- Client → BOS: SNAC(0x01,0x04) `OServiceServiceRequest{FoodGroup: 0x000E,
  TLVRestBlock{ TLV 0x01 = room's Exchange/Cookie/InstanceNumber }}`
- Server → BOS: SNAC(0x01,0x05) response as before, host/cookie now scoped
  to that specific room.
- Client opens a **second new TCP connection** for this room. Every room
  joined = one more connection; leaving = closing it.

**4. Join sequence** — after `ClientOnline` (0x01,0x02) on the new Chat
connection, the server sends, **in this exact order** (the server's own
source notes real AIM 4.0.9 breaks if reordered):
1. SNAC(0x0E,0x03) `ChatUsersJoined{Users []TLVUserInfo}` — full current
   occupant list including self, to the joining client only.
2. SNAC(0x0E,0x02) `ChatRoomInfoUpdate` — room metadata, joiner only.
3. SNAC(0x0E,0x03) `ChatUsersJoined{Users: [self]}` — broadcast to everyone
   *except* the joiner (the actual join notification).

Leaving: SNAC(0x0E,0x04) `ChatUsersLeft{Users []TLVUserInfo}` broadcast to
remaining occupants when a session disconnects.

**5. Room messaging is a separate SNAC family from ICBM:**
- Client → Chat: SNAC(0x0E,0x05) `ChatChannelMsgToHost{Cookie u64; Channel
  u16; TLVRestBlock}` — message TLV 0x05 wraps sub-TLVs (0x01 text, 0x02
  encoding, 0x03 lang). Optional whisper-target TLV.
- Server → Chat (broadcast, or single recipient if whisper): SNAC(0x0E,0x06)
  `ChatChannelMsgToClient` — server strips the payload to text/encoding/lang
  and re-adds a sender-info TLV so recipients know who sent it. Structurally
  distinct from ICBM's SNAC(0x04,0x06)/(0x04,0x07) — this is not the same
  message pipeline as 1-on-1 IMs at all.

**6. Room invites go through ICBM, not Chat/ChatNav at all.** They're ICBM
*rendezvous* proposals — channel 2, which this client has never implemented
(everything built so far is channel 1, plain IM). The proposal carries a
`CapChat` capability UUID and the room's exchange/cookie/instance as
payload; the invitee's client parses it and independently runs the ChatNav →
redirect → Chat-connect flow above to join.

## What implementing this would actually require

1. A ChatNav connection type (ephemeral, closed after room lookup/creation).
2. An `OscarSession`-equivalent per joined chat room — its own FLAP/SNAC read
   loop, its own occupant-list state, managed as a sibling to the existing
   BOS-owning session, not a replacement for it.
3. Generalizing the existing BOS-redirect/cookie-handoff logic already in
   `oscar-rs/src/client.rs`'s `login()` so it's reusable for both new
   redirect targets (ChatNav and per-room Chat), rather than being BOS-only.
4. ICBM channel-2 (rendezvous) parsing in `oscar-rs/src/messaging.rs`, just
   to receive an invite in the first place.
5. On the Tauri/frontend side: new commands for room lookup/create/join/
   send/leave, a way to manage multiple concurrent room sessions (analogous
   to how `session_actor.rs` manages the one BOS session today, but N of
   them), new events per room, and real UI (room browser/creator, a
   multi-party room screen distinct from the 1-on-1 `ImScreen`, invite
   accept/decline).

None of this is implemented. Revisit this doc if chat rooms come back into
scope — the wire formats and connection flow above should still be accurate
against Open OSCAR Server unless its own implementation changes.
