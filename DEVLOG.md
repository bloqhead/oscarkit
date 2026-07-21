# Development log

A chronological record of real bugs found (mostly against a real, self-hosted
Open OSCAR Server, not the fake ones the test suite stands up) and how they
were root-caused and fixed. Moved out of `README.md` once it grew past the
point of being a useful project overview — see that file for the current
state of the project; this one is the history of how it got there.

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

## Update: three more real bugs — the `UserInfo` block

Ran two real, simultaneous Tauri instances against the same Open OSCAR
Server (two screen names, added as each other's buddies) to test presence,
messaging, and away-info in both directions at once. Sending a message
reported success on both sides, but **nothing ever showed up as an incoming
message** — the next real-server bug, and a more structural one than the
challenge-response fix.

**The bug, in one shape, hitting three places:** every SNAC that embeds a
buddy's identity — presence arrival/departure (family 0x03), the sender of
an incoming ICBM message (family 0x04), and a Locate user-info reply (family
0x02) — was assumed to be a plain length-prefixed name immediately followed
by ordinary TLVs. Checked against Open OSCAR Server's `wire.TLVUserInfo`:
that's wrong. The real shape is name, then a **raw 2-byte warning level (not
a TLV)**, then a **TLV *count*** (not a byte length like everywhere else in
this codebase), then that many TLVs:

```go
func (s *Session) TLVUserInfo() wire.TLVUserInfo {
    return wire.TLVUserInfo{
        ScreenName:   s.DisplayScreenName().String(),
        WarningLevel: s.Warning(),
        TLVBlock:     wire.TLVBlock{TLVList: s.userInfo()}, // TLVBlock = 2-byte TLV count prefix
    }
}
```

Skipping straight from the name into `Tlv::parse_all` (what this code did
before) means the first 4 bytes it reads as a bogus TLV type+length are
actually the warning level and TLV count — enough to desync every read after
it. For the incoming-message case specifically, that desync was severe
enough that the message TLV (0x02) was never found, so the message was
silently dropped rather than displayed — exactly what surfaced as "sent
successfully but never received."

**The fix:** added `oscar_rs::UserInfo` (in `snac.rs`, next to `Tlv` — this
block is generic enough to live alongside the other shared wire-format
helpers, not tied to any one SNAC family) with a `parse` method that
consumes name, warning level, and exactly `count` TLVs, returning how many
bytes it took so the caller can find whatever comes after. Used it in:

- **`feedbag.rs`** — presence arrival/departure. Also fixed the away-flag
  TLV number itself: it was `0x0C` (guessed), should be `0x01`
  ("user flags", bit `0x0020`) or `0x06` ("status", a `u32`, bit
  `0x00000001`) depending on server — now checks both. Warning level no
  longer comes from a nonexistent "TLV 0x0A"; it's `UserInfo.warning_level`
  directly.
- **`messaging.rs`** — the actual reported bug: `parse_incoming_im` now
  parses the sender as a `UserInfo` block before looking for the message
  TLVs that follow it.
- **`locate.rs`** — same reply-parsing bug, plus a *second*, unrelated bug
  in the outgoing request: `SNAC_0x02_0x05_LocateUserInfoQuery` isn't TLVs
  at all (raw `u16` request-type field, then a BUF screen name — TLV-type
  first was also backwards vs. what this code sent), and the request was
  asking for bit `0x0001` (profile) instead of `0x0002`
  ("unavailable"/away) — so `request_user_info` was structurally wrong *and*
  requesting the wrong thing.

`oscar-rs/tests/session_integration.rs`'s fake server encoded all three
wrong assumptions and has been corrected to match the real shape throughout.

**Confirmed working against a real Open OSCAR Server**: login, `send_message`
(the *send* path), `set_away_message`. **Still to verify for real** now that
the parsing bugs above are fixed: incoming messages, presence/warning
display, buddy-info lookup, and the block list.

## Update: the buddy list was never actually syncing — two more bugs, in `FeedbagItem`

Retested with two real, simultaneous accounts after the `UserInfo` fix
above. Still no online status, buddies added in one session vanished on the
next login, and messages still didn't arrive. All three traced back to the
same place: **the buddy list has never once successfully synced against a
real server**, from the very first real-server test onward. Two bugs, both
silent (no error surfaced anywhere):

**Bug 1 — wrong reply subtype.** This code listened for the buddy-list
reply on subtype `0x05`. Checked against Open OSCAR Server's
`wire.Feedbag*` subtype constants: the real reply subtype is `0x06`. `0x05`
is a *different* client-to-server message (`FeedbagQueryIfModified`) this
client never even sends. Since the dispatch match on `REPLY` never fired,
every real feedbag reply silently fell through to the no-op default case —
`self.buddies` has only ever reflected optimistic local state (buddies
added this session), never anything the server actually confirmed.

**Bug 2 — wrong name-length prefix in `FeedbagItem`.** Confirmed against
`wire.FeedbagItem`:

```go
type FeedbagItem struct {
    Name    string `oscar:"len_prefix=uint8"`
    GroupID uint16
    ItemID  uint16
    ClassID uint16
    TLVBlock
}
```

This code encoded the name with a 2-byte length prefix; the real format
uses **one byte**, same as every other name field in the protocol. A server
reading a `FeedbagItem` this code sent would read only the first (always
`0x00`) byte as the *entire* name length — every buddy this client ever
tried to add went over the wire as a zero-length name followed by garbage.
That's the direct explanation for adds not persisting: the server had every
reason to reject or mangle the insert. `attributes` had the same class of
bug (a raw byte-length-prefixed blob instead of `TLVBlock`'s TLV-*count*
prefix + actual TLVs) — currently harmless since this codebase always sends
empty attributes, but fixed for correctness anyway (`FeedbagItem.attributes`
is now `Vec<Tlv>`, not `Vec<u8>`).

**The fix:** corrected `REPLY`/`USE` to `0x06`/`0x07` in `feedbag.rs`, fixed
`FeedbagItem::encode`/`parse` to use a one-byte name length and proper
`TLVBlock` attributes, and added `Tlv::parse_n` (in `snac.rs`) — a bounded
"parse exactly N TLVs" helper both `FeedbagItem` and `UserInfo` now share,
since both embed this same TLV-*count*-prefixed shape.

Between this and the `UserInfo` fix above, presence and incoming messages
should now actually work — both were silently starved by a buddy list that
was never really there. Still to verify for real: does this actually fix
presence/messages now, and the block list (which reuses the now-fixed
`FeedbagItem` encoding, so is very likely to have shared bug 2, but hasn't
been separately confirmed).

## Update: "Bug 2" above was itself wrong — reverted to a 2-byte name length

Added `eprintln!`-based wire logging (`[oscar-rs]` prefix — every incoming
SNAC's family/subtype/body, every outgoing feedbag insert, both hex-dumped)
since guessing further without visibility into actual bytes wasn't working.
First real capture immediately settled the question the "Bug 2" section
above got wrong: `FeedbagItem`'s name length prefix is **2 bytes**, not the
1 byte that fix changed it to — the `len_prefix=uint8` reading of Open OSCAR
Server's source was incorrect (a bad WebFetch summary, not verified against
actual bytes at the time).

The real captured reply body contained two items back to back — a
`"Buddies"` group and a `"catmints"` buddy. Both a 1-byte and a 2-byte
length read `"Buddies"` correctly (its length happens to be small enough
that the high byte of a `u16` length is `0x00`, making the two encodings
look identical for that one item — the exact ambiguity that let the wrong
fix pass a superficial glance). `"catmints"` is what broke the tie: under
the 1-byte reading, its name decoded as empty with `08` (part of the real
2-byte length `00 08`) left over as garbage, cascading into nonsense
`group_id`/`item_id`/`class_id` values for the rest of the item — visible
directly in the logged output as `(class_id: 25705, name: "", group_id:
1858, item_id: 30052)`. Under a 2-byte reading, every field of both items
decodes cleanly, including the trailing 4-byte timestamp landing exactly on
the end of the body with zero leftover bytes.

That corruption cascaded further than just a bad buddy-list read: since the
existing `"Buddies"` group didn't match anything recognizable, `group_id()`
concluded no such group existed yet and tried to *insert a new one* — a
duplicate the server evidently didn't tolerate, closing the connection
outright (surfacing as `connection closed` in the UI, and `session actor is
not running` on the next command once the actor had already torn itself
down in response).

**The fix:** reverted `FeedbagItem`'s name length back to `u16`/2 bytes.
Added `oscar-rs/src/feedbag.rs`'s `feedbag_item_parse_all_decodes_a_real_server_reply`
test using this exact captured body as a permanent ground-truth regression
test — real bytes from a real server beat a secondhand source-code summary,
so this is now the test suite's strongest evidence for this struct's shape.

The broader lesson, worth stating plainly: WebFetch-summarized source code
is a strong *lead*, not a substitute for checking actual wire bytes when
they're available. Every other fix in the two updates above (the
`UserInfo` block shape, the `0x06`/`0x07` subtype correction) came from the
same kind of source lookup and hasn't yet been contradicted by a real
capture — but per this update, that's "not yet contradicted," not
"verified," until something like this debug logging confirms it directly.

## Update: messages finally work — a missing `ClientOnline` announcement

With the buddy list actually syncing, retested messaging again. Buddies now
persist and show up correctly, but sending a message still failed — this
time with a visible error instead of silence, thanks to the new logging:
the server replied with a family-0x04 (ICBM) error SNAC, error code `0x0004`.

Checked that code against `wire.ErrorCode*`: `ErrorCodeNotLoggedOn`. Both
accounts were, in fact, logged in — so this meant the server's *session
lookup* didn't consider the recipient reachable, despite a live, fully
authenticated BOS connection.

**The bug:** this client never sent `SNAC_0x01_0x02_OServiceClientOnline`
("client online", Generic family, subtype `0x02`) — a required post-login
announcement of which SNAC families/versions the client supports. Checked
`foodgroup/oservice.go`: the server's handler for this message calls
`SetSignonComplete()` and is what starts broadcasting presence to buddies.
Without it, a session sits on an open, authenticated TCP connection
indefinitely without ever being considered "fully signed on" — invisible to
buddies, and (per `ErrorCodeNotLoggedOn`) unreachable for messaging, even
though nothing about the connection itself is wrong. This also plausibly
explains lingering presence gaps beyond what the `UserInfo` fix alone
covered — presence broadcasts to buddies are gated on this message too.

**The fix:** `login()` now sends `ClientOnline` immediately after receiving
"host online," announcing versions for every family this client
implements (Generic, Locate, BuddyPresence, Messaging, Feedbag) — an
8-byte `(family, version, tool ID, tool version)` entry per family, back
to back, no count prefix (confirmed against the struct: a bare
`[]struct{...}` with no `count_prefix` tag). Both fake-server integration
tests updated to consume this extra SNAC in the login sequence.

Also worth calling out since it's easy to miss: the *error path itself*
only became visible because of the `[oscar-rs]` debug logging added the
update before this one — before that, a rejected send just silently
"succeeded" from this client's perspective (the SNAC transmits fine over
TCP; only the server's *reply* carries the failure, and nothing was reading
or surfacing replies to `send_message` at all). That gap — actions that can
fail server-side with no client-visible signal — is worth keeping in mind
for anything not yet wired to check for a reply.

## Update: online status still wasn't showing — screen names aren't case-sensitive

Messages work now, but buddies still showed as offline in both directions.
The `[oscar-rs]` logs showed presence arrivals actually arriving
(`family=0x0003 subtype=0x0b`) and decoding cleanly via `UserInfo::parse`
— so the frame was received and parsed correctly, and the bug had to be in
what happens *after* that.

**The bug:** the arrival for one account named the buddy `"Catmints"`
(mixed case), but that same buddy's entry in the local `buddies` list —
populated from the earlier feedbag-reply capture — was `"catmints"` (all
lowercase). Every buddy lookup in this codebase (`set_online`, `set_away`,
`set_warning_level`, the Locate reply handler, `remove_buddy`,
`add_to_block_list`/`remove_from_block_list`) used a plain Rust `==` on
screen names. `"catmints" != "Catmints"`, so `.find()` silently came back
empty and every one of those setters no-op'd — a buddy could be present in
the list and still never have `is_online` (or `is_away`, or
`warning_level`, or `away_message`, or `is_blocked`) actually updated.

OSCAR screen names are canonically case- *and* whitespace-insensitive —
this isn't a server bug or an edge case, it's routine: different parts of
the protocol have no obligation to agree on which display form (a user's
own preferred capitalization vs. some normalized storage form) they hand
back.

**The fix:** added `oscar_rs::client::screen_names_match` (normalizes by
stripping whitespace and lowercasing, then compares) and replaced every
screen-name `==`/`!=` across `feedbag.rs` and `locate.rs` with it.
`set_online` also gained an `eprintln!` for the "no matching buddy found"
case, so a future mismatch like this one is visible immediately instead of
silently swallowed. `session_integration.rs`'s presence-arrival test now
deliberately sends `"BUDDY1"` against a `"Buddy1"` feedbag entry, so this
exact class of bug has a permanent regression test.

This is the last of the "buddy list looked fine but presence silently
didn't propagate" family of bugs — between this, the `UserInfo` fix, the
`FeedbagItem` fixes, and `ClientOnline`, real-server testing has now
touched every piece of the feedbag/presence/messaging/locate path at least
once. **Confirmed working end to end against a real Open OSCAR Server**:
login, buddy add/persist, presence (online/away, both directions), and
messaging (send and receive). **Still to verify for real**: ICBM warning
(`send_warning`) and the feedbag block list — the underlying bugs those
paths shared with everything above are fixed, but neither has been
separately exercised live yet.

## Update: warning and the block list — one more privacy-mode gap

Warning and blocking both tested live: warning levels update and persist
correctly, and a blocked buddy stays blocked across logout/login (so
`FeedbagItem`'s block-list encoding is confirmed good). One real gap
remained: **messages from a blocked user still got through.**

Checked Open OSCAR Server's relationship-computation SQL: whether the deny
list is consulted *at all* is gated on a separate privacy-mode preference
— a `CLASS_PDINFO` (`0x0004`) feedbag item carrying a one-byte `pdMode`
value (TLV `0x00CA`; `4` = "deny some", i.e. block only who's on the deny
list). Without that item present, the mode defaults to no enforcement —
the deny list can be fully populated and still block nobody, which is
exactly what was observed. This client never created or set it.

**Attempted fix, reverted:** added an `ensure_deny_mode_active` step to
`add_to_block_list` that found-or-created the account's `CLASS_PDINFO` item
with `pdMode = 4`. Against a real server this reliably **hard-disconnected
the connection** — no error reply, no `FeedbagStatus` ack, nothing. Checked
the server's own systemd journal at the exact moment of disconnect: also
nothing — no panic, no error, not even a connection-closed line. That
absence is itself informative: it means there's no way to tell, from
either side, *why* this specific insert is rejected. Tried a second
variant (a non-empty `name` field, matching real-client convention, in
case an empty string was the issue) — same result.

Continuing to guess at undocumented byte-level details with zero
verification signal (no packet capture, no server log, source code that
doesn't show an obvious rejection path) risks introducing more bugs than
it fixes — which is exactly what happened here: blocking *worked*
(cosmetically, if not fully enforced) before this attempt, and only
started hard-disconnecting because of it. Reverted `ensure_deny_mode_active`
entirely; `add_to_block_list`/`remove_from_block_list` are back to their
previously-verified behavior (block-list membership persists correctly
across login, per the real-server test earlier in this update).

**Net state:** login, buddy list, presence, messaging, warning, and
block-list *membership* are all confirmed working live. Block-list
*enforcement* (actually preventing messages from someone you've blocked)
remains a known, real gap — the privacy-mode-preference theory above is
still the most likely explanation, backed by the server's own SQL, but
implementing it needs an actual packet capture of a real client doing this
to get the byte format right, the same standard every other fix in this
project has been held to. Guessing further without that isn't worth the
risk of breaking working functionality again.

## Update: found it — `item_id` was the actual crash trigger, not the TLV

Open OSCAR Server is itself open source, so instead of guessing further,
checked its own test suite for a concrete example. `foodgroup/feedbag_test.go`
has a "set privacy mode" test that inserts a `CLASS_PDINFO` item with
**everything left at zero/empty defaults** — `item_id: 0`, no name, no
attributes. That's different from both earlier attempts, which used this
codebase's usual auto-incremented (non-zero) `item_id`.

Sent a diagnostic probe matching their test exactly (`item_id: 0`, no
attributes) — no TLV, so no privacy mode actually set by itself, purely a
test of whether the base insert survives. It did: the server round-tripped
it cleanly with a `FeedbagStatus` success reply (`0x0000`), the first
reply of *any* kind this specific item class had ever gotten. That isolates
`item_id` as the actual problem in both earlier crashes — the `pdMode` TLV
was never the issue.

**Attempted fix, also reverted:** `ensure_deny_mode_active` came back
(found-or-updates the account's `CLASS_PDINFO` item), now always using
`item_id: 0` instead of an auto-incremented one, with the `pdMode` TLV
attached either via a fresh `INSERT_ITEM` (no local item yet) or an
`UPDATE_ITEM` on the now-existing bare one from the probe above. Tested
live — see the next update. Spoiler: still crashes, for a different,
now conclusively isolated reason.

## Update: conclusively isolated — any TLV on a Pdinfo item crashes this server

The bare `item_id: 0` probe (no attributes) round-tripped cleanly, twice.
The exact same item shape with **one** TLV attribute attached — tried as
both `INSERT_ITEM` and `UPDATE_ITEM` — hard-disconnected immediately both
times, with zero reply of any kind. Same test, only variable changed: this
conclusively isolates the problem to *attaching any attribute at all* to a
`CLASS_PDINFO` item, independent of `item_id`, independent of insert vs.
update, independent of which specific TLV type/value was guessed.

This lines up with something noted earlier and initially treated as a
minor detail: Open OSCAR Server's own test suite never exercises a
`Pdinfo` item *with* attributes, only ever a bare one. In hindsight that's
a real signal, not a coincidence — this may well be a genuine gap in the
server's own handling of this item class, not a client-side byte-format
mistake at all. Either way, there's no more guessing left to responsibly
try: no packet capture available, no working example anywhere in the
server's own source or tests, and two independent failure modes already
ruled out by direct experiment.

**Reverted for good this time**: `ensure_deny_mode_active` and everything
related to it (the `CLASS_PDINFO` constant, the TLV constants) removed
entirely. `add_to_block_list`/`remove_from_block_list` are back to the
simple, repeatedly-confirmed-working version: deny-list membership only,
persists correctly across login, no privacy-mode manipulation attempted.

**Final state of the block list, for now:** membership (add/remove/persist)
is fully verified working. Enforcement (actually stopping a blocked user's
messages) is a known, well-diagnosed, *not* client-fixable-by-guessing gap
— worth raising with the Open OSCAR Server project directly (their
Discord, or a GitHub issue, are the natural next step) rather than
continuing to probe blind from this side.

## Update: the real 6-screen UI, sound effects, and a Linux release pipeline

Replaced the throwaway placeholder `App.vue` with the real 6-screen
retro-AIM design (Sign On, Buddy List, IM, Buddy Info, Away Message,
Preferences), wired to the Tauri bridge via a single `useSession`
composable. Added a GitHub Actions release workflow (auto patch-bump on
every push to `main`, parallel Linux/Windows/macOS builds via
`tauri-action`, one shared GitHub Release per version). Along the way, a
few more real bugs surfaced — this time in the packaging/platform layer
rather than the protocol:

**`libappindicator3-dev` vs. `libayatana-appindicator3-dev` conflict in
CI.** Ubuntu's apt refused to install both — they're mutually exclusive
forks of the same tray-icon library. Fixed by only installing the Ayatana
one, matching what Tauri v2 actually needs.

**Fatal crash on launch on Nobara/NVIDIA/Wayland**: `Gdk-Message: Error 71
(Protocol error) dispatching to Wayland display`, immediately on start.
Same root cause as the earlier `npm run tauri dev` workaround
(`WEBKIT_DISABLE_DMABUF_RENDERER=1`), but that was only ever set ad hoc for
the dev command — never baked into the shipped app. Fixed by setting it
unconditionally (Linux only) at the top of `run()` in `src-tauri/src/lib.rs`,
before the webview initializes.

**Silent sounds — a genuine upstream WebKitGTK bug.** Every sound effect
failed with no visible error. Root-caused via `GST_DEBUG` output (after
ruling out the sandbox, the codec, and the files themselves — all
confirmed fine independently via direct `gst-launch` testing): WebKitGTK's
custom-protocol handler on Linux doesn't support HTTP Range requests,
which `<audio>`/`<video>` resource loading requires. Every sound file
referenced by URL and served through Tauri's embedded-asset protocol
failed with a WebKit-level `FormatError` before GStreamer ever got to
decode it. This is a known, upstream WebKitGTK limitation
([tauri-apps/tauri#3725](https://github.com/tauri-apps/tauri/issues/3725)),
not fixable via Tauri config. Fixed by embedding sounds as base64 `data:`
URIs (`src/assets/soundData.ts`) instead of referencing them by path —
`data:` URIs resolve in-memory with no protocol/Range layer involved,
sidestepping the bug entirely.

**`WindowControls` covering the in-app back button.** After removing the
native OS window chrome (`decorations: false`, for an edge-to-edge look)
and adding a custom drag-region/minimize/close strip, that strip was
absolutely-positioned over the top of the phone-frame content — including
every screen's own back button underneath it. Fixed by making it a normal
flex child that reserves its own space above the frame instead of
overlaying it.
