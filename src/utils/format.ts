import type { Buddy } from '../types';

export type BuddyStatus = 'online' | 'away' | 'offline';

// Three tiers, not the mockup's four — there's no idle-time tracking
// anywhere in the backend, so "idle" is never a reachable state here.
export function buddyStatus(buddy: Buddy): BuddyStatus {
  if (!buddy.is_online) return 'offline';
  if (buddy.is_away) return 'away';
  return 'online';
}

export function statusLabel(buddy: Buddy): string {
  const status = buddyStatus(buddy);
  if (status === 'offline') return 'Offline';
  if (status === 'away') return buddy.away_message ? `Away – ${buddy.away_message}` : 'Away';
  return 'Online';
}

// warning_level is permille (0-1000); displayed as a percentage with one decimal.
export function formatWarningLevel(level: number): string {
  return `${(level / 10).toFixed(1)}%`;
}

export function initials(name: string): string {
  const letters = name.replace(/[^a-zA-Z]/g, '');
  return letters.slice(0, 2).toUpperCase() || '?';
}

export function formatTimestamp(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}
