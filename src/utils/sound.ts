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
  if (audioCache[event]) return audioCache[event];
  const audio = new Audio(SOUND_PATHS[event]);
  // Decode/network failures surface here (the media element's own error
  // event), separately from — and often instead of — a play() rejection.
  audio.addEventListener('error', () => {
    console.error(`[sound] "${event}" (${SOUND_PATHS[event]}) failed to load:`, audio.error);
  });
  audioCache[event] = audio;
  return audio;
}

export function playSound(event: SoundEvent): void {
  const audio = getAudio(event);
  audio.currentTime = 0;
  // Decorative — a blocked/failed playback shouldn't ever surface to the
  // user as an error toast, but log it: WebKitGTK forwards console
  // messages to stderr when the app is run from a terminal, which is the
  // only way to see *why* playback failed (autoplay policy vs. missing
  // codec vs. a bad asset path all fail silently otherwise).
  audio.play().catch((e) => console.error(`[sound] "${event}" (${SOUND_PATHS[event]}) failed to play:`, e));
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
      .catch((e) => console.error(`[sound] unlock of "${event}" (${SOUND_PATHS[event]}) failed:`, e));
  }
}
