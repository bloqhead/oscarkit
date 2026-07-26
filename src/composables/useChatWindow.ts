import { ref } from 'vue';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import type { ChatOccupant, ChatRoomSnapshot, Message } from '../types';

// Same "own separate JS runtime per window" reasoning useImWindow.ts already
// documents — this module is correct as module-scope singleton state
// precisely because each room window is its own fresh copy of it.
const roomName = ref('');
const occupants = ref<ChatOccupant[]>([]);
const messages = ref<Message[]>([]);
const myScreenName = ref('');
const closed = ref(false);

let roomLabel = '';
let seededCount = 0;
let initialized = false;

// Unlike IM (no on-demand snapshot command, so a freshly-opened window has
// to be seeded via a one-time hub-pushed event), a chat room has its own
// backend actor from the moment it's created — `get_chat_snapshot` can just
// be pulled directly once this window's `chat-update` listener is
// registered, no seed-event handshake needed.
export async function initChatWindow(label: string): Promise<void> {
  if (initialized) return;
  initialized = true;
  roomLabel = label;

  listen<ChatRoomSnapshot>('chat-update', (event) => {
    applySnapshot(event.payload);
  });

  const snapshot = await invoke<ChatRoomSnapshot>('get_chat_snapshot', { roomLabel: label });
  applySnapshot(snapshot);
}

function applySnapshot(snapshot: ChatRoomSnapshot): void {
  roomName.value = snapshot.room_name;
  occupants.value = snapshot.occupants;
  myScreenName.value = snapshot.my_screen_name;
  closed.value = snapshot.closed;

  // messages is the room's cumulative, server-received-only log (the
  // backend never echoes what this client sends — see
  // ChatRoomSession::send_message's doc comment) — append-diff against it
  // past what we've already appended, same idiom useImWindow.ts uses for
  // incoming_messages, so a locally optimistic sent-message push (below)
  // never gets clobbered by a later snapshot that doesn't know about it.
  if (snapshot.messages.length > seededCount) {
    const arrivals = snapshot.messages.slice(seededCount);
    for (const msg of arrivals) {
      messages.value.push({ from: msg.from, text: msg.text, timestamp: Date.now(), direction: 'in' });
    }
    seededCount = snapshot.messages.length;
  }
}

async function sendChat(text: string): Promise<void> {
  await invoke('send_chat_message', { roomLabel, text });
  // Optimistic local display — the backend never echoes what we sent (see
  // ChatRoomSession::send_message's doc comment for why).
  messages.value.push({ from: myScreenName.value, text, timestamp: Date.now(), direction: 'out' });
}

async function leaveRoom(): Promise<void> {
  await invoke('leave_room', { roomLabel });
}

export function useChatWindow() {
  return { roomName, occupants, messages, myScreenName, closed, sendChat, leaveRoom };
}
