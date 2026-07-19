# oscarkit

A cross-platform OSCAR protocol client for a self-hosted [Open OSCAR
Server](https://github.com/mk6i/open-oscar-server) — retro AIM functionality,
modern desktop app.

## Pivot notice

This repo originally started as a native SwiftUI/iOS client. That direction
is on hold (no working Mac to build for), and the project has moved to
**Tauri**: a Rust backend + Vue 3/TypeScript frontend, targeting Linux,
Windows, and macOS as a single lightweight desktop app rather than bundling
a full Chromium runtime the way Electron does. The Swift work isn't lost
knowledge — the protocol design (SNAC families, TLV structure, login state
machine) carries over directly, just re-implemented in Rust.

## Why Rust for the backend, specifically

Tauri's webview (the part that would run Vue) can't open raw TCP sockets —
that's a fundamental browser sandboxing restriction, not a Tauri limitation.
So the entire OSCAR protocol implementation has to live in the Rust backend
process, with the Vue frontend talking to it only through Tauri's IPC layer:
`invoke()` calls for outbound actions (login, send message, set away
status), and emitted events for things the server pushes at us (incoming
messages, presence changes, buddy list updates).

## What's here so far

- **`oscar-rs/src/flap.rs`** — the 6-byte FLAP framing header everything
  rides on top of. `FlapFrame::encode()` / `FlapFrame::parse_header()`.
- **`oscar-rs/src/snac.rs`** — the SNAC (family, subtype) command structure,
  plus TLV encoding, which is how nearly every OSCAR payload is actually
  built. `SnacFamily` currently covers Generic, Locate, BuddyPresence,
  Messaging, Feedbag, and Authorization — matches what the original Swift
  scaffold had worked out.

Both modules have unit tests (`cargo test`) covering encode/decode
round-trips — these are internally self-consistent (encoding something and
parsing it back gives you the same data), but **not yet verified against a
real server**, same caveat as the Swift scaffold before it. The actual
protocol byte layouts came from documentation and cross-client convention,
not a live capture — a Wireshark comparison against Pidgin talking to your
Open OSCAR Server instance is still the way to confirm these are exactly
right once there's a full login flow to test.

## What's not here yet

The pixel-matched 6-screen UI (Sign On, Buddy List, IM, Buddy Info, Away,
Preferences) — see "Tauri shell + IPC bridge" below for what's landed instead
as groundwork.

## Building

```bash
cargo test -p oscar-rs   # protocol crate's unit + integration tests
npm install
npm run tauri dev        # launches the actual desktop app (needs webview
                          # system libraries — see the Tauri prerequisites
                          # for your OS if this fails to compile)
```

On Linux, `npm run tauri dev` needs `webkit2gtk`/`gtk3`/`glib`/`gstreamer` `-devel` packages
(with matching `pkg-config` files) to even *compile*, separate from whatever's
already installed at runtime — see [Tauri's Linux
prerequisites](https://tauri.app/start/prerequisites/#linux) if `cargo build`
fails with `pkg-config`/GStreamer linker errors. On NVIDIA + Wayland setups,
WebKitGTK's DMA-BUF renderer can also crash the window on launch (`Error 71
(Protocol error) dispatching to Wayland display`) — work around it with
`WEBKIT_DISABLE_DMABUF_RENDERER=1 npm run tauri dev`.

This repo is now a Cargo workspace: `oscar-rs/` is the protocol library
(`cargo test -p oscar-rs` runs its tests without needing any frontend/Tauri
setup at all), `src-tauri/` is the Tauri app crate, and the Vue+TS frontend
lives in `src/` at the repo root — the same shape `npm create tauri-app`
generates.

## Update: connection layer + login (client.rs, connection.rs, server_address.rs)

The client can now actually complete a full login — connect, auth
handshake, BOS handoff — end to end.

- **`oscar-rs/src/server_address.rs`** — user-configurable server address
  parsing. Accepts a bare host, `host:port`, or an `oscar://host:port` URL
  (scheme is cosmetic, OSCAR isn't URL-addressed on the wire). This exists so
  the app can offer a "server address" field instead of a hardcoded host —
  anyone self-hosting Open OSCAR Server should be able to point this at
  their own instance, not just Daryn's.
- **`oscar-rs/src/connection.rs`** — Tokio-based `FlapConnection`. Worth
  noting vs. the Swift version: Tokio's `read_exact` reads "the next N
  bytes" directly, so there's no manual buffer-and-drain loop the way
  `NWConnection`'s callback-based reads needed. Same protocol, simpler
  implementation.
- **`oscar-rs/src/client.rs`** — the full login state machine: auth key challenge →
  MD5-roasted password (`roast_password`, chained MD5 per libpurple
  convention) → BOS handoff → wait for "host online". Returns an
  `OscarSession` holding the live BOS connection, ready for messaging/buddy
  list/away-status calls once those are ported next.

### Actually verified this time, not just compiled

`oscar-rs/tests/login_integration.rs` stands up a **fake OSCAR server locally**
(auth + BOS, both on `127.0.0.1`) and runs the real `login()` function
against it — genuinely exercising the async state machine end to end,
including a rejected-login path. This proves the code connects, round-trips
four SNACs, and completes without deadlocking. It does **not** prove the
byte layouts match a real server's expectations — that still needs a
Wireshark capture against Pidgin talking to the actual Hetzner box, same
caveat as everywhere else in this project. 23 tests total, `cargo test`
passes clean with no warnings.

### Not yet ported

The actual Tauri shell (`invoke` commands, event emission) and the Vue
frontend — everything else from the Swift scaffold has landed, see below.

## Update: feedbag, presence, away status, and messaging ported to Rust (feedbag.rs, locate.rs, messaging.rs)

Ports the three Swift files that existed on top of the login scaffold —
`Feedbag.swift`, `AwayStatus.swift`, and the messaging bits of
`OSCARClient.swift` — onto `OscarSession`. `login()` now calls
`request_buddy_list()` automatically once BOS comes online, matching what
real clients do before anything else becomes meaningful.

- **`oscar-rs/src/feedbag.rs`** — `FeedbagItem` (encode/parse for the
  buddy-list wire format) and `Buddy` (the UI-friendly projection:
  online/away state plus group membership). `OscarSession::request_buddy_list`,
  `add_buddy`, `remove_buddy`; frame handlers for the feedbag reply/ack cycle
  and the Buddy-family (0x03) presence arrivals/departures.
- **`oscar-rs/src/locate.rs`** — away status rides the Locate family (0x02),
  same quirk as the Swift version: there's no dedicated "go away"/"come
  back" command, setting a non-empty away message *is* going away.
  `set_away_message`, `request_user_info`, and the reply handler that
  updates a buddy's `away_message` in place.
- **`oscar-rs/src/messaging.rs`** — `send_message` builds the ICBM send-IM
  SNAC (cookie, channel, recipient BUF, nested message TLV); `IncomingIm`
  plus the parser for the mirror-image incoming structure.
- **`OscarSession::handle_next_frame`** (in `oscar-rs/src/client.rs`) — the dispatch loop
  tying it together: reads one FLAP frame off BOS, routes it by SNAC family
  to the feedbag/presence/locate/messaging handler, mutating `buddies`,
  `incoming_messages`, `away_message` in place. This is also the natural
  spot for a future Tauri layer to poll from and re-emit as frontend events.

One thing that *did* change vs. the Swift version, not just a port: there's
no `guard case .online = state` check scattered through every method
anymore. In Rust, holding an `OscarSession` at all already proves login
succeeded and BOS is connected — the type system does the job the runtime
state enum did in Swift.

`oscar-rs/tests/session_integration.rs` extends the fake-server approach from
`login_integration.rs` to this layer: a scripted fake BOS server drives a
full round — buddy-list sync and ack, a presence arrival with the away bit
set, an incoming IM, an outgoing reply, setting an away message, and a
user-info round trip — and asserts the client's state (`buddies`,
`incoming_messages`, `away_message`) ends up correct. Same caveat as ever:
this proves the state machine and wire encoding are internally consistent,
not that they match a real server — that's still a Wireshark-capture-against-Pidgin
task for whenever there's a live Hetzner box to point this at.

## Update: Tauri shell + IPC bridge + warn/block-list (workspace restructure)

This repo is now a Cargo workspace (`oscar-rs/` protocol crate + `src-tauri/`
app crate + a root-level Vue/TS frontend in `src/`) with an actual, working
Tauri command/event bridge — plus two protocol features the eventual UI
needs that hadn't been ported yet: ICBM warning and feedbag permit/deny
(block list).

- **`oscar-rs/src/connection.rs`** — `FlapConnection` can now be split via
  `into_split()` into a `FlapReader`/`FlapWriter` pair (built on Tokio's
  `OwnedReadHalf`/`OwnedWriteHalf`). This exists so a background task can own
  the read half exclusively while the write half stays with `OscarSession` —
  necessary for the Tauri actor below.
- **`oscar-rs/src/client.rs`** — `OscarSession::handle_next_frame` is now
  layered on `split_reader()` (hands out the `FlapReader`, once) and
  `dispatch_frame` (the actual per-SNAC dispatch, now callable directly with
  a frame from wherever it was read). Also gained `pending_warnings`, a
  `request_id → screen_name` map so an ICBM warning reply — which carries no
  screen name — can still be attributed back to the right buddy.
- **`oscar-rs/src/feedbag.rs`** — `add_to_block_list`/`remove_from_block_list`
  (feedbag `CLASS_DENY` items, same insert/delete-item mechanism as
  add/remove buddy) plus a `set_warning_level` helper. `Buddy` gained
  `warning_level: u16` (populated from presence-arrival TLV `0x0A`, same
  0-1000 permille scale used everywhere) and `is_blocked: bool` (from
  `CLASS_DENY` membership at feedbag-sync time).
- **`oscar-rs/src/messaging.rs`** — `send_warning` (ICBM family 0x04,
  subtype 0x08) and the subtype-0x09 reply handler, using `pending_warnings`
  for attribution since the reply itself is screen-name-less.
- **`src-tauri/src/session_actor.rs`** — the actor that owns a live
  `OscarSession` on its own task after `login` succeeds. Frontend commands
  never touch the session directly; they send a `SessionCommand` (with a
  `oneshot` reply channel) over an `mpsc` channel the actor's `select!` loop
  picks up. A **separate** dedicated task owns the connection's read half
  and forwards parsed frames over its own channel — this two-task split
  matters: `FlapReader::read_frame`'s underlying socket read is not
  cancellation-safe, so racing it directly inside the actor's `select!`
  against incoming commands could silently desync the FLAP stream if a
  command arrived mid-read. After every processed command or frame, the
  actor emits a `session-update` event with a full `SessionSnapshot`
  (buddies, incoming messages, away message) for the frontend to consume.
- **`src-tauri/src/commands.rs`** — the `#[tauri::command]` surface:
  `login`, `send_message`, `add_buddy`, `remove_buddy`, `set_away_message`,
  `request_user_info`, `send_warning`, `add_to_block_list`,
  `remove_from_block_list`. All but `login` forward into the running
  session's actor and await a per-call `Result<(), String>`.
- **`src/App.vue`** — intentionally bare-bones: a sign-on form, a buddy list
  with online/away/warning%/blocked state and per-buddy action buttons, an
  incoming-message log, and forms for sending messages and setting an away
  message. This is throwaway scaffolding to prove the bridge works, not the
  final design — the real pixel-matched 6-screen retro-AIM UI (Sign On,
  Buddy List, IM, Buddy Info, Away, Preferences) is a separate follow-up
  pass.

### Verification split, same honesty as always

`cargo test -p oscar-rs` (29 tests, including new integration coverage for
the warn/block-list round trip) and `npm run build` (`vue-tsc` + `vite
build`) both pass clean in this dev environment. `cargo check -p oscarkit`
(the Tauri crate) could **not** be verified here — this sandbox has
webview runtime libraries but not the `-devel`/`pkg-config` headers Tauri's
build scripts need even to compile (confirmed missing, and not installable
without root). That check, and the actual `npm run tauri dev` /
real-Open-OSCAR-Server login this bridge exists for, are the next things to
run on a machine with the full Tauri prerequisites installed.

## Update: first real-server login — a real bug, found and fixed

`npm run tauri dev` finally ran against a real, self-hosted Open OSCAR Server
instance (not the fake ones the test suite stands up) — and it immediately
surfaced a genuine byte-layout bug that no amount of internally-consistent
fake-server testing could have caught, exactly the caveat this README has
been repeating since the very first commit.

**The bug:** `login()`'s auth-key/challenge-response parsing (family 0x17,
subtype 0x07) assumed the body was a TLV block and looked for a "TLV 0x01"
containing the challenge string. Every real server rejected this — the
error was always `unexpected or malformed response: auth key TLV (0x01)
missing from challenge reply`.

**The fix:** checked Open OSCAR Server's own Go source
(`wire.SNAC_0x17_0x07_BUCPChallengeResponse`) instead of guessing again —
turns out this specific reply is *not* a TLV at all:

```go
type SNAC_0x17_0x07_BUCPChallengeResponse struct {
    AuthKey string `oscar:"len_prefix=uint16"`
}
```

Just a plain 2-byte big-endian length prefix followed by that many bytes of
challenge string — no type field, no TLV framing. `oscar-rs/src/client.rs`
now parses it that way directly. Everything *else* checked against the same
source turned out to already be correct: the outgoing challenge request (TLV
0x01 = screen name) and the login response (TLVs 0x01/0x05/0x06/0x08 for
screen name/BOS address/cookie/error code) are genuinely TLV-based and
matched what this codebase already assumed. `oscar-rs/tests/login_integration.rs`'s
fake server was encoding the same wrong assumption and has been corrected to
match.

**Confirmed working against a real Open OSCAR Server**, not just the fake
ones in the test suite: login (full auth handshake + BOS handoff),
`send_message` (ICBM), and `set_away_message` (Locate). **Still unverified
against a real server**: buddy-list sync/presence, ICBM warning, and the
feedbag block list — those are the next things to exercise for real.
