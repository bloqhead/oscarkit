// Files live in public/sounds/ (see the README there) and are served as
// static assets by Vite's public/ convention — no import needed, just a
// stable /sounds/<name> path.
const SOUND_PATHS = {
  arrive: '/sounds/buddyin.mp3',
  depart: '/sounds/buddyout.mp3',
  message: '/sounds/imrcv.mp3',
} as const;

export type SoundEvent = keyof typeof SOUND_PATHS;

const audioCache: Partial<Record<SoundEvent, HTMLAudioElement>> = {};

function getAudio(event: SoundEvent): HTMLAudioElement {
  return audioCache[event] ?? (audioCache[event] = new Audio(SOUND_PATHS[event]));
}

export function playSound(event: SoundEvent): void {
  const audio = getAudio(event);
  audio.currentTime = 0;
  // Decorative — a blocked/failed playback shouldn't ever surface as an error.
  audio.play().catch(() => {});
}

// WebKitGTK (the Linux webview) requires a real user gesture in the call
// stack before it'll allow audio.play() at all — our actual triggers are
// async Tauri event listeners reacting to presence/message changes, never a
// click. Call this once from inside a genuine click handler (Sign On) to
// prime every sound; a successful play+immediate-pause during a real
// gesture unlocks playback for the rest of the session.
export function unlockAudio(): void {
  for (const event of Object.keys(SOUND_PATHS) as SoundEvent[]) {
    const audio = getAudio(event);
    audio
      .play()
      .then(() => audio.pause())
      .catch(() => {});
  }
}
