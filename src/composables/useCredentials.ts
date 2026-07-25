import { ref } from 'vue';
import { load, type Store } from '@tauri-apps/plugin-store';
import { invoke } from '@tauri-apps/api/core';

// Non-secret (server address, screen name): a plain local settings file via
// tauri-plugin-store. The password itself never goes anywhere near this —
// see save_password/get_saved_password/delete_saved_password in
// src-tauri/src/credentials.rs, which use the OS-native credential store
// (Keychain/Credential Manager/Secret Service) instead.
const SETTINGS_FILE = 'settings.json';

const savedServer = ref('');
const savedScreenName = ref('');

let storePromise: Promise<Store> | null = null;

function getStore(): Promise<Store> {
  if (!storePromise) storePromise = load(SETTINGS_FILE);
  return storePromise;
}

async function loadSavedCredentials(): Promise<void> {
  const store = await getStore();
  savedServer.value = (await store.get<string>('server')) ?? '';
  savedScreenName.value = (await store.get<string>('screenName')) ?? '';
}

async function saveCredentials(server: string, screenName: string): Promise<void> {
  const store = await getStore();
  await store.set('server', server);
  await store.set('screenName', screenName);
  await store.save();
  savedServer.value = server;
  savedScreenName.value = screenName;
}

async function getSavedPassword(screenName: string): Promise<string | null> {
  try {
    return await invoke<string | null>('get_saved_password', { screenName });
  } catch {
    // No saved password, or the OS credential store isn't reachable right
    // now (e.g. no secret-service daemon running) — either way, just fall
    // back to an empty password field rather than surfacing an error.
    return null;
  }
}

async function savePassword(screenName: string, password: string): Promise<void> {
  await invoke('save_password', { screenName, password });
}

async function deleteSavedPassword(screenName: string): Promise<void> {
  await invoke('delete_saved_password', { screenName });
}

export function useCredentials() {
  return {
    savedServer,
    savedScreenName,
    loadSavedCredentials,
    saveCredentials,
    getSavedPassword,
    savePassword,
    deleteSavedPassword,
  };
}
