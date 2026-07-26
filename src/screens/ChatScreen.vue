<script setup lang="ts">
import { nextTick, ref, watch } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useChatWindow } from '../composables/useChatWindow';
import { formatTimestamp } from '../utils/format';
import { escapeMessageText, sanitizeFormattedMessage } from '../utils/sanitizeFormattedMessage';
import TitleBar from '../components/TitleBar.vue';

const { roomName, occupants, messages, sendChat } = useChatWindow();

const messageText = ref('');
const messageListEl = ref<HTMLDivElement | null>(null);

function closeWindow(): void {
  // Routes through the same onCloseRequested handler ChatWindow.vue
  // registers (which leaves the room before actually closing) — close()
  // fires that event, it isn't bypassed the way destroy() would be.
  getCurrentWindow().close();
}

watch(messages, () => {
  nextTick(() => {
    if (messageListEl.value) messageListEl.value.scrollTop = messageListEl.value.scrollHeight;
  });
}, { deep: true });

async function handleSend(): Promise<void> {
  const raw = messageText.value.trim();
  if (!raw) return;
  messageText.value = '';
  await sendChat(escapeMessageText(raw));
}
</script>

<template>
  <div class="chat-screen">
    <TitleBar :title="roomName" :show-back="true" @back="closeWindow" />

    <div class="occupant-strip">
      {{ occupants.length }} {{ occupants.length === 1 ? 'person' : 'people' }}:
      <span v-for="(o, idx) in occupants" :key="o.screen_name">{{ o.screen_name }}{{ idx < occupants.length - 1 ? ', ' : '' }}</span>
    </div>

    <div ref="messageListEl" class="message-list">
      <div v-for="(msg, idx) in messages" :key="idx" class="message-line">
        <span class="from" :class="msg.direction === 'out' ? 'me' : 'them'">{{ msg.from }}:</span>
        <span class="text" v-html="sanitizeFormattedMessage(msg.text)"></span>
        <span class="time">{{ formatTimestamp(msg.timestamp) }}</span>
      </div>
    </div>

    <form class="send-row" @submit.prevent="handleSend">
      <input v-model="messageText" class="text-input" type="text" placeholder="Type a message…" />
      <button class="btn-gold" type="submit">Send</button>
    </form>
  </div>
</template>

<style scoped>
.chat-screen {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: #fff;
}

.occupant-strip {
  padding: 4px 12px;
  font-size: 11px;
  color: #777;
  border-bottom: 1px solid #eee;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.message-list {
  flex: 1;
  overflow-y: auto;
  padding: 8px 12px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.message-line {
  font-size: 13px;
  line-height: 1.4;
}

.from {
  font-weight: 700;
  margin-right: 4px;
}

.from.me {
  color: var(--badge-red);
}

.from.them {
  color: var(--color-name-online);
}

.time {
  margin-left: 6px;
  font-size: 10px;
  color: #aaa;
}

.send-row {
  display: flex;
  gap: 8px;
  padding: 8px 12px;
  border-top: 1px solid #eee;
}

.send-row .text-input {
  flex: 1;
}
</style>
