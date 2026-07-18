# OSCARKit — scaffold

A from-scratch Swift implementation of the client side of the OSCAR protocol,
targeting a self-hosted [Open OSCAR Server](https://github.com/mk6i/open-oscar-server)
instance rather than the (long dead) real AIM network.

## What's here

- **FLAP.swift** — the 6-byte framing header everything rides on top of.
- **SNAC.swift** — the (family, subtype) command structure + TLV encoding, which
  is how basically every OSCAR payload is actually built.
- **FLAPConnection.swift** — an `NWConnection`-based socket that buffers partial
  reads and emits complete `FLAPFrame`s.
- **OSCARClient.swift** — the actual login state machine:
  1. connect to the auth server, FLAP hello
  2. request an auth key (SNAC 0x17/0x06) with your screen name
  3. server sends back a challenge (0x17/0x07)
  4. compute `MD5(authKey + MD5(password) + "AOL Instant Messenger (SM)")`
     and send it as the login request (0x17/0x02)
  5. server responds (0x17/0x03) with a BOS server address + session cookie
  6. disconnect from auth, connect to BOS, FLAP hello carrying the cookie
  7. once "host online" (0x01/0x03) arrives, you're live
  8. send/receive plain-text IMs (family 0x04)
- **ChatDemoView.swift** — a bare SwiftUI view proving the plumbing works.
  Move this into your actual app target — it currently lives next to the
  library code for convenience, which you don't want long-term (a library
  target generally shouldn't import SwiftUI).

## What's deliberately NOT here yet

Scoped out on purpose so the login + basic IM path could get done first:

- **Rate limiting negotiation** (family 0x01, subtypes 0x06/0x07). Real AIM
  clients always do this before anything else. Open OSCAR Server is lenient
  about skipping it, but if you see mystery disconnects, this is the first
  thing to add.
- **Buddy list / presence** (family 0x03). No roster, no online/offline status.
- **Away messages, warnings, buddy icons, chat rooms, file transfer.**
- **Error recovery** — the FLAP reader bails out on a malformed frame instead
  of trying to resync. Fine for a v0.1, not fine for production.

## Before this will actually compile and connect

1. Add this as a Swift Package to an Xcode project (or `swift build` a
   command-line target first — much faster iteration loop than a full app
   while you're debugging the wire protocol).
2. Point `OSCARClient.init(host:)` at your Open OSCAR Server's
   `OSCAR_ADVERTISED_LISTENERS_PLAIN` value.
3. If your server is on WAN mode with an actual domain, `NWConnection` will
   just work. If it's LAN-only with a bare IP, same deal — no changes needed.

## How to verify the protocol details are actually right

The honest caveat: I wrote this from protocol documentation and libpurple's
well-known OSCAR behavior, but I have not run it against a live server —
your network sandbox doesn't allow reaching your VPS or home server. A few
things worth double-checking once you have Open OSCAR Server running:

- **Capture real Pidgin traffic with Wireshark** while it logs into your
  server. Compare frame-by-frame against what OSCARKit sends — this is the
  fastest way to catch a wrong TLV type or byte offset.
- The **exact byte layout of the ICBM send/receive message body** (the nested
  TLV-inside-TLV "fragment" structure) is the single trickiest part of this
  protocol and the place most homebrew clients get subtly wrong first. If
  messages don't show up, check this first.
- **DISABLE_AUTH**: while `DISABLE_AUTH=true` (the Open OSCAR Server default),
  literally any password produces a valid login, so the MD5 roasting math
  matters less for testing — you'll want to flip it to `false` deliberately
  to confirm the hash computation is actually correct and not just being
  ignored by the server.

## Suggested next milestone

Get `login()` reaching `.online` state against your real server, then get
one IM sent and echoed back in a second client (Pidgin is the easiest ground
truth). Everything else — buddy list, retro UI chrome, away messages — is
much easier to add once that loop is proven out.

## Update: buddy list (Feedbag.swift)

Added the roster layer on top of the login/messaging scaffold:

- **Fetching**: `requestBuddyList()` is called automatically once `state` hits
  `.online`, mirroring what real clients do — roster comes down before
  anything else is treated as ready.
- **`buddies`** (published) is the UI-friendly view — screen name, group,
  online status. **`feedbagItems`** is the raw synced roster, kept around so
  add/remove can look up existing group/item IDs without re-fetching.
- **Presence** (family 0x03, arrival/departure) is handled separately from
  the roster itself — two different SNAC families cooperating to produce one
  buddy list UI.
- `addBuddy(screenName:toGroup:)` creates the group on the fly if it doesn't
  exist yet, then inserts the buddy — two feedbag inserts under the hood.

**Same caveat as everything else here**: the FEEDBAG_REPLY body layout
(version byte + item count + items + trailing timestamp) is written from
protocol documentation, not verified against live traffic. If your buddy
list comes back empty or garbled after `requestBuddyList()`, this is the
first place to check with a Wireshark capture of Pidgin's own feedbag sync.
