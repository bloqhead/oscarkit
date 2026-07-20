// Mirrors screen_names_match in oscar-rs/src/client.rs — OSCAR screen names
// are canonically case/whitespace-insensitive, confirmed against the real
// server. Used to key message threads and unread counts and to match a
// buddy's screen_name against an IncomingIm.from.
export function normalizeScreenName(name: string): string {
  return name.trim().toLowerCase().replace(/\s+/g, '');
}
