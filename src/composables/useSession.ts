import { computed, reactive, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Buddy, GroupedBuddies, Message, Screen, SessionSnapshot, Toast } from '../types';
import { normalizeScreenName } from '../utils/screenName';
import { playSound } from '../utils/sound';

// Module-scope singleton state — every useSession() call shares the same
// instance. No Pinia needed at this app's size.

const currentScreen = ref<Screen>('signon');
const activeBuddy = ref<string | null>(null);
const infoBuddy = ref<string | null>(null);
const infoReturnScreen = ref<'im' | 'buddylist'>('buddylist');

const snapshot = ref<SessionSnapshot | null>(null);
const errorMessage = ref<string | null>(null);

const messageThreads = reactive<Record<string, Message[]>>({});
const unreadCounts = reactive<Record<string, number>>({});
const toasts = reactive<Toast[]>([]);
const soundPrefs = reactive({
  buddySignOn: true,
  buddySignOff: true,
  imReceived: true,
  imSent: true,
  idleReminder: true,
  welcome: true,
  goodbye: true,
});

let toastSeq = 0;

function pushToast(kind: Toast['kind'], text: string): void {
  const id = ++toastSeq;
  toasts.push({ id, kind, text });
  setTimeout(() => dismissToast(id), 3000);
}

function dismissToast(id: number): void {
  const idx = toasts.findIndex((t) => t.id === id);
  if (idx !== -1) toasts.splice(idx, 1);
}

// `snapshot.value` is reassigned wholesale on every session-update event
// (never mutated in place), so Vue's watch callback receives the true
// previous snapshot as `oldSnap` for free — no separate "previous" ref needed.
watch(snapshot, (newSnap, oldSnap) => {
  if (!newSnap) return;

  if (oldSnap) {
    for (const buddy of newSnap.buddies) {
      const prev = oldSnap.buddies.find(
        (b) => normalizeScreenName(b.screen_name) === normalizeScreenName(buddy.screen_name),
      );
      // A buddy present in newSnap but absent from oldSnap is a feedbag
      // roster change, not a sign-on — don't toast that.
      if (prev && prev.is_online !== buddy.is_online) {
        const kind = buddy.is_online ? 'arrive' : 'depart';
        pushToast(kind, buddy.screen_name);
        if (buddy.is_online ? soundPrefs.buddySignOn : soundPrefs.buddySignOff) playSound(kind);
      }
    }
  }

  // incoming_messages only ever grows, so a length comparison is always the
  // correct "did anything new arrive" test, and slice(prevCount) is always
  // exactly the new tail.
  const prevCount = oldSnap ? oldSnap.incoming_messages.length : 0;
  if (newSnap.incoming_messages.length > prevCount) {
    const arrivals = newSnap.incoming_messages.slice(prevCount);
    for (const im of arrivals) {
      const key = normalizeScreenName(im.from);
      const isNewThread = !messageThreads[key];
      const thread = messageThreads[key] ?? (messageThreads[key] = []);
      thread.push({ from: im.from, text: im.text, timestamp: Date.now(), direction: 'in' });

      const isViewingThisThread =
        currentScreen.value === 'im' &&
        activeBuddy.value !== null &&
        normalizeScreenName(activeBuddy.value) === key;

      // On the very first snapshot after login (oldSnap undefined), threads
      // are seeded but nothing is toasted/counted — those messages predate
      // the UI watching for them.
      if (oldSnap && !isViewingThisThread) {
        unreadCounts[key] = (unreadCounts[key] ?? 0) + 1;
        pushToast('message', `New IM from ${im.from}`);
        // A brand-new conversation rings distinctly from a message arriving
        // in one you've already got open elsewhere.
        if (soundPrefs.imReceived) playSound(isNewThread ? 'newchat' : 'message');
      }
    }
  }
});

listen<SessionSnapshot>('session-update', (event) => {
  snapshot.value = event.payload;
});

listen<string>('session-error', (event) => {
  errorMessage.value = event.payload;
  resetSessionState();
});

// Shared by logout() and the session-error listener so a disconnect or an
// explicit sign-off never leaves the next login seeing a previous user's
// stale threads/unread counts.
function resetSessionState(): void {
  snapshot.value = null;
  currentScreen.value = 'signon';
  activeBuddy.value = null;
  infoBuddy.value = null;
  for (const key of Object.keys(messageThreads)) delete messageThreads[key];
  for (const key of Object.keys(unreadCounts)) delete unreadCounts[key];
}

// Every backend action below reports failure as an error toast (in addition
// to rethrowing, so callers can still skip a subsequent step like navigating
// away on failure) — previously a rejected invoke() just vanished as an
// unhandled promise rejection with no visible feedback.
async function guarded<T>(action: () => Promise<T>, failureText: string): Promise<T> {
  try {
    return await action();
  } catch (e) {
    pushToast('error', `${failureText}: ${String(e)}`);
    throw e;
  }
}

const groupedBuddies = computed<GroupedBuddies[]>(() => {
  const buddies = snapshot.value?.buddies ?? [];
  const byGroup = new Map<string, Buddy[]>();
  for (const buddy of buddies) {
    if (!byGroup.has(buddy.group_name)) byGroup.set(buddy.group_name, []);
    byGroup.get(buddy.group_name)!.push(buddy);
  }
  return [...byGroup.entries()].map(([name, members]) => ({
    name,
    online: members.filter((b) => b.is_online).length,
    total: members.length,
    buddies: members,
  }));
});

function getBuddy(screenName: string): Buddy | undefined {
  return snapshot.value?.buddies.find(
    (b) => normalizeScreenName(b.screen_name) === normalizeScreenName(screenName),
  );
}

function getThread(screenName: string): Message[] {
  return messageThreads[normalizeScreenName(screenName)] ?? [];
}

function unreadFor(screenName: string): number {
  return unreadCounts[normalizeScreenName(screenName)] ?? 0;
}

function goToBuddyList(): void {
  currentScreen.value = 'buddylist';
}

function goToIm(screenName: string): void {
  activeBuddy.value = screenName;
  unreadCounts[normalizeScreenName(screenName)] = 0;
  currentScreen.value = 'im';
}

function goToInfo(screenName: string, from: 'im' | 'buddylist'): void {
  infoBuddy.value = screenName;
  infoReturnScreen.value = from;
  currentScreen.value = 'info';
}

function backFromInfo(): void {
  currentScreen.value = infoReturnScreen.value;
}

function goToAway(): void {
  currentScreen.value = 'away';
}

function goToPreferences(): void {
  currentScreen.value = 'preferences';
}

async function login(server: string, screenName: string, password: string): Promise<void> {
  errorMessage.value = null;
  try {
    const result = await invoke<SessionSnapshot>('login', { server, screenName, password });
    snapshot.value = result;
    currentScreen.value = 'buddylist';
    if (soundPrefs.welcome) playSound('signOn');
  } catch (e) {
    errorMessage.value = String(e);
    throw e;
  }
}

async function logout(): Promise<void> {
  await invoke('logout');
  if (soundPrefs.goodbye) playSound('signOff');
  resetSessionState();
}

async function sendIm(recipient: string, text: string): Promise<void> {
  await guarded(() => invoke('send_message', { recipient, text }), "Couldn't send message");
  // The backend never echoes what we sent, so this is the only way sent
  // messages end up in a thread. Only reached if the send above succeeded.
  const key = normalizeScreenName(recipient);
  const thread = messageThreads[key] ?? (messageThreads[key] = []);
  thread.push({ from: snapshot.value!.screen_name, text, timestamp: Date.now(), direction: 'out' });
  if (soundPrefs.imSent) playSound('sent');
}

async function addBuddy(screenName: string, groupName: string): Promise<void> {
  await guarded(() => invoke('add_buddy', { screenName, groupName }), "Couldn't add buddy");
}

async function removeBuddy(screenName: string): Promise<void> {
  await guarded(() => invoke('remove_buddy', { screenName }), "Couldn't remove buddy");
}

async function setAway(text: string | null): Promise<void> {
  await guarded(() => invoke('set_away_message', { text }), "Couldn't update away message");
}

async function clearAway(): Promise<void> {
  await setAway(null);
}

async function requestInfo(screenName: string): Promise<void> {
  await guarded(() => invoke('request_user_info', { screenName }), "Couldn't request buddy info");
}

async function warnBuddy(screenName: string, anonymous: boolean): Promise<void> {
  await guarded(() => invoke('send_warning', { screenName, anonymous }), "Couldn't send warning");
}

async function toggleBlock(buddy: Buddy): Promise<void> {
  await guarded(
    () =>
      buddy.is_blocked
        ? invoke('remove_from_block_list', { screenName: buddy.screen_name })
        : invoke('add_to_block_list', { screenName: buddy.screen_name }),
    "Couldn't update block list",
  );
}

export function useSession() {
  return {
    currentScreen,
    activeBuddy,
    infoBuddy,
    snapshot,
    errorMessage,
    toasts,
    soundPrefs,

    groupedBuddies,
    getBuddy,
    getThread,
    unreadFor,

    goToBuddyList,
    goToIm,
    goToInfo,
    backFromInfo,
    goToAway,
    goToPreferences,

    login,
    logout,
    sendIm,
    addBuddy,
    removeBuddy,
    setAway,
    clearAway,
    requestInfo,
    warnBuddy,
    toggleBlock,

    dismissToast,
  };
}
