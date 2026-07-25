import { ref } from 'vue';
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';
import { getCurrentWindow } from '@tauri-apps/api/window';

const permissionGranted = ref(false);

async function ensurePermission(): Promise<void> {
  let granted = await isPermissionGranted();
  if (!granted) {
    const permission = await requestPermission();
    granted = permission === 'granted';
  }
  permissionGranted.value = granted;
}

// Native OS notifications are for when you're not looking at the app — the
// in-app toasts (ToastContainer) already cover the focused case, and firing
// both for the same event would be redundant.
async function notify(title: string, body: string): Promise<void> {
  if (!permissionGranted.value) return;
  try {
    const focused = await getCurrentWindow().isFocused();
    if (focused) return;
    sendNotification({ title, body });
  } catch {
    // Best-effort — a failed native notification shouldn't affect the app.
  }
}

export function useNotifications() {
  return { permissionGranted, ensurePermission, notify };
}
