import { computed, reactive, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { emitTo, listen } from '@tauri-apps/api/event';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import type { Buddy, ChatInvite, GroupedBuddies, ImSeedPayload, Message, Screen, SessionSnapshot, Toast } from '../types';
import { normalizeScreenName } from '../utils/screenName';
import { playSound } from '../utils/sound';
import { useNotifications } from './useNotifications';

const { notify } = useNotifications();

// Module-scope singleton state — every useSession() call shares the same
// instance. No Pinia needed at this app's size. This is the hub only —
// each open conversation is a separate OS window/JS runtime with its own
// thin useImWindow() state; see that file for why hub state can't just be
// shared across windows.

const currentScreen = ref<Screen>('signon');
const infoBuddy = ref<string | null>(null);

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

// Which conversations currently have an open OS window, keyed by the same
// label the window was created with. Plain (not reactive) — only ever read/
// written from functions below, nothing templated renders it directly.
const openImWindows = new Set<string>();
// Same idea, one entry per open chat room window, keyed by the room label
// the backend assigned (session_actor::room_label_for).
const openChatWindows = new Set<string>();

// Invites the user has declined or already accepted, so they stop appearing
// in `pendingInvites` even though the backend's own
// `incoming_chat_invites` vector still has the entry (declining is a pure
// frontend-side dismissal — see declineInvite below for why no backend call
// exists for it, and acceptInvite for why a backend-vector *index* isn't a
// safe identity to hold onto across renders: accepting shifts every later
// invite's index down by one, silently mislabeling whatever was declined
// under a now-stale index). Keyed by content instead — see inviteKey.
const dismissedInviteKeys = new Set<string>();

function inviteKey(invite: ChatInvite): string {
  return `${normalizeScreenName(invite.from)}::${invite.room.room_cookie}`;
}

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

function labelFor(buddyName: string): string {
  return `im-${normalizeScreenName(buddyName)}`;
}

// Opens a real OS window for this conversation (mirroring classic AIM: every
// conversation is its own movable, independently-closable window), or
// focuses the existing one if already open.
async function openImWindow(buddyName: string): Promise<void> {
  const label = labelFor(buddyName);
  unreadCounts[normalizeScreenName(buddyName)] = 0;

  const existing = await WebviewWindow.getByLabel(label);
  if (existing) {
    await existing.setFocus();
    return;
  }

  const win = new WebviewWindow(label, {
    url: `/#/im/${encodeURIComponent(buddyName)}`,
    title: buddyName,
    width: 340,
    height: 480,
    decorations: false,
    resizable: true,
  });
  openImWindows.add(label);
  win.once('tauri://destroyed', () => {
    openImWindows.delete(label);
  });
}

// Opens (or focuses) a room's OS window. `label` is the opaque
// backend-assigned room label (session_actor::room_label_for) returned by
// create_room/accept_chat_invite — used verbatim as both the window label
// and the room-scoped Tauri commands' `roomLabel` argument, so there's no
// separate frontend-side label derivation to keep in sync with the backend
// the way `labelFor` has to for IM/buddy names.
async function openChatWindow(label: string, roomName: string): Promise<void> {
  const existing = await WebviewWindow.getByLabel(label);
  if (existing) {
    await existing.setFocus();
    return;
  }

  const win = new WebviewWindow(label, {
    url: `/#/chat/${encodeURIComponent(label)}`,
    title: roomName,
    width: 380,
    height: 520,
    decorations: false,
    resizable: true,
  });
  openChatWindows.add(label);
  win.once('tauri://destroyed', () => {
    openChatWindows.delete(label);
  });
}

// Answers a freshly-opened IM window's im-window-ready announcement with a
// one-time snapshot of what it needs to boot. Necessary because emit/listen
// is live pub-sub, not a durable queue — the window's own listen() only
// sees events fired after it registers, and there's no snapshot-on-demand
// command, so without this a new window would open blank until some
// unrelated future event happened to arrive. See useImWindow.ts.
listen<{ label: string }>('im-window-ready', (event) => {
  const label = event.payload.label;
  const slug = label.startsWith('im-') ? label.slice(3) : label;
  const buddy = getBuddy(slug);
  if (!buddy || !snapshot.value) return;
  const seed: ImSeedPayload = { buddy, thread: getThread(slug), myScreenName: snapshot.value.screen_name };
  emitTo(label, 'im-seed', seed);
});

// An IM window's own thread dies when that window closes, so it reports
// what it sent back here for durable storage — otherwise reopening a
// closed conversation would show what the other person said but not your
// own side of it. Also where the "sent" sound stays, same as every other
// sound decision — centralized in the hub, never duplicated per-window.
listen<{ buddyName: string; message: Message }>('im-sent', (event) => {
  const key = normalizeScreenName(event.payload.buddyName);
  const thread = messageThreads[key] ?? (messageThreads[key] = []);
  thread.push(event.payload.message);
  if (soundPrefs.imSent) playSound('sent');
});

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
        // Native notifications are for when you're not looking at the app
        // at all (notify() itself checks window focus) — not gated by the
        // sound toggles, which are a separate concern.
        notify(buddy.screen_name, buddy.is_online ? 'Signed on' : 'Signed off');
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

      // A window already open for this buddy is treated as "you're seeing
      // this conversation live" — a coarser proxy than true per-window
      // focus tracking (which would need each IM window to report its own
      // focus state back here), but a reasonable one: once you've opened a
      // conversation, toast/sound/notification spam for it stops until you
      // close the window again.
      const hasOpenWindow = openImWindows.has(labelFor(im.from));

      // On the very first snapshot after login (oldSnap undefined), threads
      // are seeded but nothing is toasted/counted/opened — those messages
      // predate the UI watching for them.
      if (oldSnap && !hasOpenWindow) {
        unreadCounts[key] = (unreadCounts[key] ?? 0) + 1;
        pushToast('message', `New IM from ${im.from}`);
        // A brand-new conversation rings distinctly from a message arriving
        // in one you've already got open elsewhere.
        if (soundPrefs.imReceived) playSound(isNewThread ? 'newchat' : 'message');
        notify(im.from, im.text);
        // Classic AIM opens a window the moment a conversation starts, not
        // just when you click something — mirror that here.
        openImWindow(im.from);
      }
    }
  }

  // Same append-only-growth reasoning as incoming_messages above —
  // incoming_chat_invites only shrinks when *this* client accepts one
  // (backend removes it by index at that point), so between any two
  // snapshots it only ever gains entries from other people's invites.
  const prevInviteCount = oldSnap ? oldSnap.incoming_chat_invites.length : 0;
  if (oldSnap && newSnap.incoming_chat_invites.length > prevInviteCount) {
    const arrivals = newSnap.incoming_chat_invites.slice(prevInviteCount);
    for (const invite of arrivals) {
      // Not gated by soundPrefs/toasts — an invite needs an explicit
      // Accept/Decline, so it's surfaced via the persistent
      // ChatInviteBanner (App.vue), not the auto-dismissing toast stream.
      notify(invite.from, `Invited you to "${invite.room.room_name}"`);
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
  infoBuddy.value = null;
  for (const key of Object.keys(messageThreads)) delete messageThreads[key];
  for (const key of Object.keys(unreadCounts)) delete unreadCounts[key];
  dismissedInviteKeys.clear();

  // Unlike an IM window (harmless, purely local state that just goes stale
  // if left open), a chat room window's backend actor owns its own live
  // connection independent of the main session — ending the session doesn't
  // implicitly close it. Ask each open room window to close itself; that
  // triggers its own onCloseRequested handler (ChatWindow.vue), which calls
  // leave_room before actually closing, so the room's connection/actor get
  // torn down too instead of leaking.
  for (const label of [...openChatWindows]) {
    WebviewWindow.getByLabel(label).then((win) => win?.close());
  }
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

// The banner-facing view of incoming_chat_invites: whatever the backend
// still has on hand, minus whatever this client has already dismissed
// locally (declined, or accepted and already opened).
const pendingInvites = computed<ChatInvite[]>(() => {
  const invites = snapshot.value?.incoming_chat_invites ?? [];
  return invites.filter((invite) => !dismissedInviteKeys.has(inviteKey(invite)));
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
  openImWindow(screenName);
}

function goToInfo(screenName: string): void {
  infoBuddy.value = screenName;
  currentScreen.value = 'info';
}

function backFromInfo(): void {
  currentScreen.value = 'buddylist';
}

function goToAway(): void {
  currentScreen.value = 'away';
}

function goToPreferences(): void {
  currentScreen.value = 'preferences';
}

function goToCreateRoom(): void {
  currentScreen.value = 'createroom';
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

async function createRoom(roomName: string, inviteScreenNames: string[]): Promise<void> {
  const label = await guarded(
    () => invoke<string>('create_room', { roomName, inviteScreenNames }),
    "Couldn't create room",
  );
  currentScreen.value = 'buddylist';
  await openChatWindow(label, roomName);
}

async function acceptInvite(invite: ChatInvite): Promise<void> {
  // Recomputed at call time rather than captured when the invite was first
  // rendered — see dismissedInviteKeys' doc comment for why a backend-vector
  // index can't just be captured once and reused.
  const invites = snapshot.value?.incoming_chat_invites ?? [];
  const index = invites.findIndex((i) => inviteKey(i) === inviteKey(invite));
  if (index === -1) return; // already gone — e.g. dispatched twice from a double-click

  dismissedInviteKeys.add(inviteKey(invite));
  const label = await guarded(() => invoke<string>('accept_chat_invite', { index }), "Couldn't join room");
  await openChatWindow(label, invite.room.room_name);
}

function declineInvite(invite: ChatInvite): void {
  dismissedInviteKeys.add(inviteKey(invite));
}

export function useSession() {
  return {
    currentScreen,
    infoBuddy,
    snapshot,
    errorMessage,
    toasts,
    soundPrefs,

    groupedBuddies,
    getBuddy,
    getThread,
    unreadFor,
    pendingInvites,

    goToBuddyList,
    goToIm,
    goToInfo,
    backFromInfo,
    goToAway,
    goToPreferences,
    goToCreateRoom,

    login,
    logout,
    addBuddy,
    removeBuddy,
    setAway,
    clearAway,
    requestInfo,
    warnBuddy,
    toggleBlock,
    createRoom,
    acceptInvite,
    declineInvite,

    dismissToast,
  };
}
