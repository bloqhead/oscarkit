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

- **`src/flap.rs`** — the 6-byte FLAP framing header everything rides on
  top of. `FlapFrame::encode()` / `FlapFrame::parse_header()`.
- **`src/snac.rs`** — the SNAC (family, subtype) command structure, plus TLV
  encoding, which is how nearly every OSCAR payload is actually built.
  `SnacFamily` currently covers Generic, Locate, BuddyPresence, Messaging,
  Feedbag, and Authorization — matches what the original Swift scaffold had
  worked out.

Both modules have unit tests (`cargo test`) covering encode/decode
round-trips — these are internally self-consistent (encoding something and
parsing it back gives you the same data), but **not yet verified against a
real server**, same caveat as the Swift scaffold before it. The actual
protocol byte layouts came from documentation and cross-client convention,
not a live capture — a Wireshark comparison against Pidgin talking to your
Open OSCAR Server instance is still the way to confirm these are exactly
right once there's a full login flow to test.

## What's not here yet

Everything above the framing/encoding layer — the actual TCP connection
handling (Tokio `TcpStream`), the login state machine (auth key exchange,
MD5-roasted password, BOS handoff), messaging, buddy list (feedbag), away
status, and — new to this iteration — the Tauri command/event layer and the
Vue frontend itself. These existed in the Swift version and are next up to
port.

## Building

```bash
cargo test    # runs the unit tests, no Tauri/frontend setup needed for this
```

Full Tauri scaffolding (`npm create tauri-app`, frontend deps, system
webview libraries) comes once there's a connection layer worth putting a UI
in front of.

## Update: connection layer + login (client.rs, connection.rs, server_address.rs)

The client can now actually complete a full login — connect, auth
handshake, BOS handoff — end to end.

- **`server_address.rs`** — user-configurable server address parsing.
  Accepts a bare host, `host:port`, or an `oscar://host:port` URL (scheme
  is cosmetic, OSCAR isn't URL-addressed on the wire). This exists so the
  app can offer a "server address" field instead of a hardcoded host —
  anyone self-hosting Open OSCAR Server should be able to point this at
  their own instance, not just Daryn's.
- **`connection.rs`** — Tokio-based `FlapConnection`. Worth noting vs. the
  Swift version: Tokio's `read_exact` reads "the next N bytes" directly, so
  there's no manual buffer-and-drain loop the way `NWConnection`'s
  callback-based reads needed. Same protocol, simpler implementation.
- **`client.rs`** — the full login state machine: auth key challenge →
  MD5-roasted password (`roast_password`, chained MD5 per libpurple
  convention) → BOS handoff → wait for "host online". Returns an
  `OscarSession` holding the live BOS connection, ready for messaging/buddy
  list/away-status calls once those are ported next.

### Actually verified this time, not just compiled

`tests/login_integration.rs` stands up a **fake OSCAR server locally**
(auth + BOS, both on `127.0.0.1`) and runs the real `login()` function
against it — genuinely exercising the async state machine end to end,
including a rejected-login path. This proves the code connects, round-trips
four SNACs, and completes without deadlocking. It does **not** prove the
byte layouts match a real server's expectations — that still needs a
Wireshark capture against Pidgin talking to the actual Hetzner box, same
caveat as everywhere else in this project. 23 tests total, `cargo test`
passes clean with no warnings.

### Not yet ported

Messaging, buddy list (feedbag), away status — all existed in the Swift
scaffold and are straightforward ports now that `OscarSession` exists to
hang them off of. Also still pending: the actual Tauri shell (`invoke`
commands, event emission) and the Vue frontend.
