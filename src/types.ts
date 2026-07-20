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

export interface SessionSnapshot {
  screen_name: string;
  buddies: Buddy[];
  incoming_messages: IncomingIm[];
  away_message: string | null;
}

// Frontend-only types below — no backend equivalent.

export type Screen = 'signon' | 'buddylist' | 'im' | 'info' | 'away' | 'preferences';

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
