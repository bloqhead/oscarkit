// Files live in public/sounds/ (see the README there) and are served as
// static assets by Vite's public/ convention — no import needed, just a
// stable /sounds/<name> path.
const SOUND_PATHS = {
  arrive: '/sounds/buddyin.mp3',
  depart: '/sounds/buddyout.mp3',
  message: '/sounds/imrcv.mp3',
} as const;

export type SoundEvent = keyof typeof SOUND_PATHS;

export function playSound(event: SoundEvent): void {
  const audio = new Audio(SOUND_PATHS[event]);
  // Decorative — a blocked/failed playback shouldn't ever surface as an error.
  audio.play().catch(() => {});
}
