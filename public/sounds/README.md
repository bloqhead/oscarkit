# Sound effects

Sourced from an old personal AIM install. Wiring lives in
[`src/utils/sound.ts`](../../src/utils/sound.ts), triggered from the
presence/message diffing in
[`useSession.ts`](../../src/composables/useSession.ts) and gated by the
matching toggle on the Preferences screen.

**These files themselves are not what actually gets played.** WebKitGTK's
custom-protocol handler on Linux doesn't support HTTP Range requests, which
`<audio>`/`<video>` resource loading requires — any real media file
referenced by URL and served through Tauri's embedded-asset protocol fails
there with a WebKit-level `FormatError` before GStreamer ever sees it
(confirmed via `GST_DEBUG` output; this is an upstream WebKitGTK
limitation, not fixable via Tauri config — see
[tauri-apps/tauri#3725](https://github.com/tauri-apps/tauri/issues/3725)).
The seven wired-up sounds are duplicated as base64 `data:` URIs in
[`src/assets/soundData.ts`](../../src/assets/soundData.ts) (regenerate with
`base64 -w0 public/sounds/<name>.mp3` if the source files change) and
`sound.ts` imports those, not a `/sounds/<name>` path. **Any future sound
wired up here needs the same treatment**, not just a path reference.

## Wired up

| File          | Plays on                                    | Preferences toggle |
|---------------|----------------------------------------------|---------------------|
| `welcome.mp3` | you sign on                                  | Welcome sound       |
| `goodbye.mp3` | you sign off                                 | Goodbye sound       |
| `buddyin.mp3` | a buddy signs on                             | Buddy sign-on       |
| `buddyout.mp3`| a buddy signs off                            | Buddy sign-off      |
| `imrcv.mp3`   | a new IM arrives in a conversation you already have open elsewhere | IM received |
| `ring.mp3`    | a new IM arrives from someone with no existing open thread yet | IM received |
| `imsend.mp3`  | you send a message                           | IM sent             |

`welcome.mp3`/`goodbye.mp3` are **not authentic AIM** — they're the classic
AOL desktop client's own sign-on/sign-off sounds, not part of the AIM sound
pack the other five came from. Used anyway, deliberately, for the
nostalgia value — a conscious choice, not a mix-up.

## Present but not wired up

No idle-time tracking exists in this app (only three presence tiers: online/
away/offline), so there's no event to hook `idle-reminder` to — the
Preferences toggle for it is a placeholder until that's built.

`dooropen.mp3`/`doorslam.mp3` are an alternate take on the same buddy
sign-on/off concept `buddyin.mp3`/`buddyout.mp3` already cover (per the
source of this pack, likely from a different AIM version) — redundant, not
wired to anything.

The rest of this classic AIM sound pack doesn't map to anything OSCARKit
currently does — no voice/Talk feature, mail integration, or
UI-panel-switching for `talkbeg`/`talkend`/`talkstop`,
`phone`/`IncomingCall`/`PhoneRingInternal`, `newmail`, `cashregister`,
`panelchange1`, or `newalert`. `moo` is the classic hidden easter-egg sound
with no obvious place to attach it. They're harmless sitting here unused —
ask if you'd like any of them given a real trigger.
