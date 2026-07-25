import { ref } from 'vue';
import { emit, listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import type { Buddy, ImSeedPayload, Message, SessionSnapshot } from '../types';
import { normalizeScreenName } from '../utils/screenName';

// Module-scope, same singleton pattern as useSession/useUpdater/
// useNotifications — but unlike those, this is correct precisely because
// each IM window is its own separate JS runtime (a fresh copy of this whole
// module graph), so there's exactly one buddy's worth of state per window,
// never cross-window leakage.
const buddy = ref<Buddy | null>(null);
const thread = ref<Message[]>([]);
const myScreenName = ref('');

let targetBuddyName = '';
let seededCount = 0;
let initialized = false;

// A freshly-opened window's listen() only sees events fired *after* it
// registers — the hub's session-update broadcast that triggered this
// window's creation already happened before this window's webview finished
// booting. So: register listeners first, then announce readiness; the hub
// replies with a one-time snapshot (see useSession.ts's im-window-ready
// handler) of exactly what this window needs to not open blank.
export function initImWindow(buddyName: string): void {
  if (initialized) return;
  initialized = true;
  targetBuddyName = buddyName;
  const key = normalizeScreenName(buddyName);

  listen<ImSeedPayload>('im-seed', (event) => {
    if (normalizeScreenName(event.payload.buddy.screen_name) !== key) return;
    buddy.value = event.payload.buddy;
    thread.value = event.payload.thread;
    myScreenName.value = event.payload.myScreenName;
    seededCount = event.payload.thread.length;
  });

  listen<SessionSnapshot>('session-update', (event) => {
    const snap = event.payload;
    const updated = snap.buddies.find((b) => normalizeScreenName(b.screen_name) === key);
    if (updated) buddy.value = updated;

    // incoming_messages is the hub's cumulative, all-buddies log — filter to
    // this buddy, then only the tail past what we were seeded/have already
    // appended, same append-only idiom useSession.ts uses for the hub itself.
    const relevant = snap.incoming_messages.filter(
      (im) => normalizeScreenName(im.from) === key,
    );
    if (relevant.length > seededCount) {
      const arrivals = relevant.slice(seededCount);
      for (const im of arrivals) {
        thread.value.push({ from: im.from, text: im.text, timestamp: Date.now(), direction: 'in' });
      }
      seededCount = relevant.length;
    }
  });

  emit('im-window-ready', { label: getCurrentWindow().label });
}

async function sendIm(text: string): Promise<void> {
  await invoke('send_message', { recipient: targetBuddyName, text });
  const message: Message = { from: myScreenName.value, text, timestamp: Date.now(), direction: 'out' };
  // Optimistic local display — the backend never echoes what we sent.
  thread.value.push(message);
  // This window's own thread dies with it when closed, so the hub (which
  // stays alive and is what seeds any future reopen of this conversation)
  // needs to durably record this too, exactly as it already does for
  // incoming messages via session-update. See useSession.ts's im-sent
  // listener — also where the "message sent" sound decision stays, kept
  // hub-centralized like every other sound choice.
  emit('im-sent', { buddyName: targetBuddyName, message });
}

export function useImWindow() {
  return { buddy, thread, myScreenName, sendIm };
}
