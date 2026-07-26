<script setup lang="ts">
import { onMounted } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { initChatWindow, useChatWindow } from '../composables/useChatWindow';
import WindowControls from '../components/WindowControls.vue';
import ChatScreen from './ChatScreen.vue';

const props = defineProps<{ roomLabel: string }>();
const { leaveRoom } = useChatWindow();

onMounted(() => {
  initChatWindow(props.roomLabel);

  // Unlike an IM window (which needs no server-side teardown — the buddy
  // relationship isn't a live connection), closing a room window has to
  // tell the backend to leave it, or its chat_actor and connection leak
  // forever. Intercept every close path (the custom WindowControls button
  // as well as WM-level close) the same way App.vue intercepts the hub's
  // close, then destroy() (not close()) to avoid re-triggering this same
  // handler once the room's already been left.
  getCurrentWindow().onCloseRequested(async (event) => {
    event.preventDefault();
    await leaveRoom();
    await getCurrentWindow().destroy();
  });
});
</script>

<template>
  <div class="chat-window-shell">
    <WindowControls />
    <div class="frame">
      <ChatScreen />
    </div>
  </div>
</template>

<style scoped>
.chat-window-shell {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: #fff;
}

.frame {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
</style>
