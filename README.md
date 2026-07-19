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
