import { ref } from 'vue';
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

// Deliberately independent of useSession — update checks make sense before
// (or without ever) signing in, since they're about the app itself, not
// the OSCAR session.
const updateInfo = ref<Update | null>(null);
const isInstalling = ref(false);
const installError = ref<string | null>(null);

async function checkForUpdate(): Promise<void> {
  try {
    const result = await check();
    if (result) updateInfo.value = result;
  } catch {
    // A failed check (e.g. offline, GitHub unreachable) shouldn't interrupt
    // using the app — just silently try again next launch.
  }
}

async function installUpdate(): Promise<void> {
  if (!updateInfo.value) return;
  isInstalling.value = true;
  installError.value = null;
  try {
    await updateInfo.value.downloadAndInstall();
    await relaunch();
  } catch (e) {
    installError.value = String(e);
    isInstalling.value = false;
  }
}

function dismissUpdate(): void {
  updateInfo.value = null;
}

export function useUpdater() {
  return { updateInfo, isInstalling, installError, checkForUpdate, installUpdate, dismissUpdate };
}
