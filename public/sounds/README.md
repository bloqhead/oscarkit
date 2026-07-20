# Sound effects

Sourced from an old personal AIM install. Wiring lives in
[`src/utils/sound.ts`](../../src/utils/sound.ts), triggered from the
presence/message diffing in
[`useSession.ts`](../../src/composables/useSession.ts) and gated by the
matching toggle on the Preferences screen.

## Wired up

| File          | Plays on          | Preferences toggle |
|---------------|-------------------|---------------------|
| `buddyin.mp3` | a buddy signs on  | Buddy sign-on       |
| `buddyout.mp3`| a buddy signs off | Buddy sign-off      |
| `imrcv.mp3`   | a new IM arrives  | IM received         |

## Present but not wired up

No idle-time tracking exists in this app (only three presence tiers: online/
away/offline), so there's no event to hook `idle-reminder` to — the
Preferences toggle for it is a placeholder until that's built.

The rest of this classic AIM sound pack doesn't map to anything OSCARKit
currently does — no "IM sent" toggle to hook `imsend.mp3` to yet, and no
voice/Talk feature, mail integration, or UI-panel-switching for
`talkbeg`/`talkend`/`talkstop`, `phone`/`ring`/`IncomingCall`/
`PhoneRingInternal`, `newmail`, `cashregister`, `panelchange1`, `dooropen`/
`doorslam`, `newalert`, or `moo`. They're harmless sitting here unused —
ask if you'd like any of them given a real trigger (an "IM sent" toggle for
`imsend.mp3` is the most obvious candidate).
