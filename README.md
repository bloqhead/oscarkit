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

Tauri's webview (the part that runs Vue) can't open raw TCP sockets — that's
a fundamental browser sandboxing restriction, not a Tauri limitation. So the
entire OSCAR protocol implementation lives in the Rust backend process, with
the Vue frontend talking to it only through Tauri's IPC layer: `invoke()`
calls for outbound actions (login, send message, set away status), and
emitted events for things the server pushes at us (incoming messages,
presence changes, buddy list updates).

## What's here

- **`oscar-rs/`** — the OSCAR/FLAP/SNAC protocol library. Login (BUCP auth +
  BOS handoff), buddy list sync, presence, away status, messaging, ICBM
  warning, and feedbag block-list membership are all implemented and
  **confirmed working against a real, self-hosted Open OSCAR Server** (not
  just the fake local servers the test suite also uses) — see
  [`DEVLOG.md`](DEVLOG.md) for the real-server bugs that surfaced along the
  way and how each was root-caused.
- **`src-tauri/`** — the Tauri app crate: a background actor owns the live
  `OscarSession`, frontend commands dispatch into it over a channel, and
  every state change re-emits a `session-update` event for the UI to
  consume.
- **`src/`** — the Vue 3/TypeScript frontend: all 6 screens (Sign On, Buddy
  List, IM, Buddy Info, Away Message, Preferences) in a retro-AIM/Google
  Talk-style compact window, no native OS title bar. Includes sound effects
  (buddy sign-on/off, IM sent/received, your own sign-on/sign-off) and
  desktop notifications via in-app toasts.
- **`.github/workflows/release.yml`** — every push to `main` auto-bumps the
  patch version, tags it, and builds Linux (`.deb`/`.rpm`/`.AppImage`),
  Windows (`.exe`/`.msi`), and macOS (`.dmg`) installers in parallel,
  publishing all of them to one GitHub Release. See "Releasing" below.

## Known gaps

- **Feedbag block-list *enforcement*** (as opposed to membership, which
  works) is unimplemented — a blocked buddy stays blocked and persists
  correctly, but their messages still get through. Root-caused to a
  server-side privacy-mode gap in Open OSCAR Server itself; see the last
  few entries in [`DEVLOG.md`](DEVLOG.md) for the full investigation.
- **No idle-time tracking** — presence is a 3-tier system (online/away/
  offline), no idle detection. The "Idle reminder" sound toggle in
  Preferences is a placeholder with no event to trigger it yet.
- **macOS builds are Apple Silicon only** — `macos-latest` GitHub Actions
  runners default to `aarch64`; an Intel Mac build isn't currently produced.
- **No mobile builds** — Tauri v2 supports iOS/Android, and the project is
  scaffolded compatibly (`crate-type` already includes what mobile targets
  need), but nothing mobile-specific has been set up.

## Building

```bash
cargo test -p oscar-rs   # protocol crate's unit + integration tests
npm install
npm run tauri dev        # launches the actual desktop app (needs webview
                          # system libraries — see the Tauri prerequisites
                          # for your OS if this fails to compile)
```

On Linux, this needs `webkit2gtk`/`gtk3`/`glib`/`gstreamer` `-devel` packages
(with matching `pkg-config` files) to even *compile*, separate from whatever's
already installed at runtime — see [Tauri's Linux
prerequisites](https://tauri.app/start/prerequisites/#linux) if `cargo build`
fails with `pkg-config`/GStreamer linker errors. On NVIDIA + Wayland setups,
WebKitGTK's DMA-BUF renderer can also crash the window on launch — this is
already worked around unconditionally in `src-tauri/src/lib.rs`, so it
shouldn't need a manual env var for `tauri dev` either.

This repo is a Cargo workspace: `oscar-rs/` is the protocol library
(`cargo test -p oscar-rs` runs its tests without needing any frontend/Tauri
setup at all), `src-tauri/` is the Tauri app crate, and the Vue+TS frontend
lives in `src/` at the repo root — the same shape `npm create tauri-app`
generates.

To build an installable local package instead of running in dev mode:

```bash
npm run tauri build
```

Bundles land in `target/release/bundle/`. On Linux this produces `.deb`,
`.rpm`, and `.AppImage` — though `.AppImage` bundling can fail on very new
distro toolchains (`linuxdeploy`'s bundled `strip` doesn't understand newer
ELF sections some toolchains now emit); `.deb`/`.rpm` aren't affected and
this doesn't reproduce on GitHub Actions' runners.

## Releasing

Every push to `main` is a release — no manual version bump or tagging
needed. `.github/workflows/release.yml` reads the current version, bumps
the patch number, commits that back (`chore: release vX.Y.Z [skip ci]`,
which doesn't retrigger the workflow), tags it, and builds/publishes
Linux/Windows/macOS installers to a GitHub Release for that tag — all
driven by [`tauri-action`](https://github.com/tauri-apps/tauri-action). A
workflow can also be triggered manually (`workflow_dispatch`) if needed,
e.g. for the very first run after adding the workflow file itself, since
GitHub doesn't trigger a workflow on the same push that introduces it.

None of the published installers are code-signed, so Windows will show a
SmartScreen warning and macOS requires right-click → Open the first time
(Gatekeeper, unidentified developer) — expected for an unsigned build, not
a bug.

## Development history

[`DEVLOG.md`](DEVLOG.md) has the detailed, chronological record of every
real bug found against a real server (or a real OS/webview environment) and
how each was root-caused and fixed — useful if you're debugging something
that smells similar, or just curious how this got built.
