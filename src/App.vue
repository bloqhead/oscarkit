<script setup lang="ts">
import { onMounted } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { exit } from '@tauri-apps/plugin-process';
import { useSession } from './composables/useSession';
import { useUpdater } from './composables/useUpdater';
import { useNotifications } from './composables/useNotifications';
import WindowControls from './components/WindowControls.vue';
import UpdateBanner from './components/UpdateBanner.vue';
import ChatInviteBanner from './components/ChatInviteBanner.vue';
import SignOnScreen from './screens/SignOnScreen.vue';
import BuddyListScreen from './screens/BuddyListScreen.vue';
import BuddyInfoScreen from './screens/BuddyInfoScreen.vue';
import AwayMessageScreen from './screens/AwayMessageScreen.vue';
import PreferencesScreen from './screens/PreferencesScreen.vue';
import CreateRoomScreen from './screens/CreateRoomScreen.vue';

const { currentScreen } = useSession();
const { checkForUpdate } = useUpdater();
const { ensurePermission } = useNotifications();

// Both checked once at startup, independent of sign-on — these are about
// the app itself, not the OSCAR session.
onMounted(() => {
  checkForUpdate();
  ensurePermission();

  // Tauri's default is "quit once every window is closed" — now that IM
  // conversations open as their own windows, closing just the hub would
  // otherwise leave them orphaned with no way back to Buddy List (no tray
  // icon to reopen it from). Closing the hub means quitting entirely,
  // taking any open IM windows with it — this intercepts both the custom
  // WindowControls close button and WM-level close (Alt+F4/taskbar), since
  // decorations being off only affects drawn chrome, not the close protocol.
  getCurrentWindow().onCloseRequested(async (event) => {
    event.preventDefault();
    await exit(0);
  });
});
</script>

<template>
  <div class="app-shell">
    <WindowControls />
    <div class="frame-wrap">
      <div class="phone-frame">
        <UpdateBanner />
        <ChatInviteBanner v-if="currentScreen !== 'signon'" />
        <div class="screen-wrap">
          <SignOnScreen v-if="currentScreen === 'signon'" />
          <BuddyListScreen v-else-if="currentScreen === 'buddylist'" />
          <BuddyInfoScreen v-else-if="currentScreen === 'info'" />
          <AwayMessageScreen v-else-if="currentScreen === 'away'" />
          <PreferencesScreen v-else-if="currentScreen === 'preferences'" />
          <CreateRoomScreen v-else-if="currentScreen === 'createroom'" />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.app-shell {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: #2b2b2b;
}

.frame-wrap {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}

.phone-frame {
  width: 340px;
  height: 100%;
  max-height: 780px;
  background: #fff;
  box-shadow: 0 0 24px rgba(0, 0, 0, 0.5);
  overflow: hidden;
  position: relative;
  border-radius: 8px;
  display: flex;
  flex-direction: column;
}

.screen-wrap {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
</style>
