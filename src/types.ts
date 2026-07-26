// Mirrors oscar-rs/src/feedbag.rs::Buddy, oscar-rs/src/messaging.rs::IncomingIm,
// and src-tauri/src/session_actor.rs::SessionSnapshot exactly. These derive
// plain serde::Serialize with no rename_all, so field names cross the Tauri
// IPC boundary as literal snake_case — do not camelCase these.
export interface Buddy {
  screen_name: string;
  group_name: string;
  is_online: boolean;
  is_away: boolean;
  away_message: string | null;
  warning_level: number; // permille, 0-1000
  is_blocked: boolean;
}

export interface IncomingIm {
  from: string;
  text: string;
}

// Mirrors oscar-rs/src/chat_nav.rs::ChatRoomHandle.
export interface ChatRoomHandle {
  exchange: number;
  room_cookie: string;
  instance: number;
  room_name: string;
}

// Mirrors oscar-rs/src/messaging.rs::ChatInvite.
export interface ChatInvite {
  from: string;
  invitation_text: string;
  room: ChatRoomHandle;
}

// Mirrors oscar-rs/src/chat.rs::ChatOccupant.
export interface ChatOccupant {
  screen_name: string;
  warning_level: number;
}

// Mirrors oscar-rs/src/chat.rs::ChatMessage.
export interface ChatMessageWire {
  from: string;
  text: string;
}

// Mirrors src-tauri/src/chat_actor.rs::ChatRoomSnapshot.
export interface ChatRoomSnapshot {
  room_name: string;
  occupants: ChatOccupant[];
  messages: ChatMessageWire[];
  my_screen_name: string;
  closed: boolean;
}

export interface SessionSnapshot {
  screen_name: string;
  buddies: Buddy[];
  incoming_messages: IncomingIm[];
  away_message: string | null;
  incoming_chat_invites: ChatInvite[];
}

// Frontend-only types below — no backend equivalent.

// Neither 'im' nor 'chat' is a hub screen — each conversation/room is its
// own OS window (src/screens/ImWindow.vue, ChatWindow.vue), not a state
// inside this switch.
export type Screen = 'signon' | 'buddylist' | 'info' | 'away' | 'preferences' | 'createroom';

export interface Message {
  from: string;
  text: string;
  timestamp: number;
  direction: 'in' | 'out';
}

export interface Toast {
  id: number;
  kind: 'arrive' | 'depart' | 'message' | 'error';
  text: string;
}

export interface GroupedBuddies {
  name: string;
  online: number;
  total: number;
  buddies: Buddy[];
}

// Cross-window seed handshake (see useSession.ts / useImWindow.ts): a
// freshly-opened IM window's listen('session-update', ...) only sees events
// fired after it registers, so the hub replies with a one-time snapshot of
// what that window needs on boot via emitTo(label, 'im-seed', ...).
export interface ImSeedPayload {
  buddy: Buddy;
  thread: Message[];
  myScreenName: string;
}
