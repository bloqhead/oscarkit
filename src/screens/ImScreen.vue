<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { useSession } from '../composables/useSession';
import { buddyStatus, formatTimestamp, statusLabel } from '../utils/format';
import { escapeMessageText, sanitizeFormattedMessage } from '../utils/sanitizeFormattedMessage';
import TitleBar from '../components/TitleBar.vue';
import StatusDot from '../components/StatusDot.vue';

const { snapshot, activeBuddy, getBuddy, getThread, goToBuddyList, goToInfo, sendIm } = useSession();

const buddy = computed(() => (activeBuddy.value ? getBuddy(activeBuddy.value) : undefined));
const thread = computed(() => (activeBuddy.value ? getThread(activeBuddy.value) : []));
const messageText = ref('');
const messageListEl = ref<HTMLDivElement | null>(null);

const boldActive = ref(false);
const italicActive = ref(false);
const underlineActive = ref(false);
const colorActive = ref(false);
const messageColor = ref('#c62828');

watch(thread, () => {
  nextTick(() => {
    if (messageListEl.value) messageListEl.value.scrollTop = messageListEl.value.scrollHeight;
  });
}, { deep: true });

// Whole-message formatting, not per-character — the compose box stays a
// plain <input> rather than a contenteditable rich-text editor. Whatever's
// active wraps the entire message in the corresponding HTML tag; oscar-rs
// carries this through to the wire completely transparently (it's just
// text bytes to the protocol), so real AIM clients on the other end will
// render it as formatted too.
function applyFormatting(text: string): string {
  let wrapped = escapeMessageText(text);
  if (underlineActive.value) wrapped = `<u>${wrapped}</u>`;
  if (italicActive.value) wrapped = `<i>${wrapped}</i>`;
  if (boldActive.value) wrapped = `<b>${wrapped}</b>`;
  if (colorActive.value) wrapped = `<font color="${messageColor.value}">${wrapped}</font>`;
  return wrapped;
}

async function handleSend(): Promise<void> {
  const raw = messageText.value.trim();
  if (!raw || !activeBuddy.value) return;
  messageText.value = '';
  await sendIm(activeBuddy.value, applyFormatting(raw));
}
</script>

<template>
  <div class="im-screen">
    <TitleBar :title="activeBuddy ?? ''" :show-back="true" @back="goToBuddyList">
      <template #leading>
        <StatusDot v-if="buddy" :status="buddyStatus(buddy)" />
      </template>
      <template #trailing>
        <button
          class="info-btn"
          aria-label="Buddy Info"
          @click="activeBuddy && goToInfo(activeBuddy, 'im')"
        >
          i
        </button>
      </template>
    </TitleBar>
    <div v-if="buddy" class="status-line">{{ statusLabel(buddy) }}</div>

    <div ref="messageListEl" class="message-list">
      <div v-for="(msg, idx) in thread" :key="idx" class="message-line">
        <span class="from" :class="msg.direction === 'out' ? 'me' : 'them'">
          {{ msg.direction === 'out' ? snapshot?.screen_name : msg.from }}:
        </span>
        <span class="text" v-html="sanitizeFormattedMessage(msg.text)"></span>
        <span class="time">{{ formatTimestamp(msg.timestamp) }}</span>
      </div>
    </div>

    <div class="format-row">
      <button
        class="fmt-btn"
        type="button"
        :class="{ active: boldActive }"
        aria-label="Bold"
        @click="boldActive = !boldActive"
      >
        B
      </button>
      <button
        class="fmt-btn"
        type="button"
        :class="{ active: italicActive }"
        aria-label="Italic"
        @click="italicActive = !italicActive"
      >
        I
      </button>
      <button
        class="fmt-btn"
        type="button"
        :class="{ active: underlineActive }"
        aria-label="Underline"
        @click="underlineActive = !underlineActive"
      >
        U
      </button>
      <button
        class="fmt-btn swatch"
        type="button"
        :class="{ active: colorActive }"
        :style="colorActive ? { background: messageColor } : undefined"
        aria-label="Toggle text color"
        @click="colorActive = !colorActive"
      />
      <input
        v-if="colorActive"
        v-model="messageColor"
        type="color"
        class="color-input"
        aria-label="Pick text color"
      />
    </div>

    <form class="send-row" @submit.prevent="handleSend">
      <input v-model="messageText" class="text-input" type="text" placeholder="Type a message…" />
      <button class="btn-gold" type="submit">Send</button>
    </form>
  </div>
</template>

<style scoped>
.im-screen {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: #fff;
}

.info-btn {
  width: 22px;
  height: 22px;
  border-radius: 50%;
  border: 1px solid #fff;
  background: transparent;
  color: #fff;
  font-family: var(--font-aim);
  font-style: italic;
  font-weight: 700;
  font-size: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.status-line {
  padding: 4px 12px;
  font-size: 11px;
  color: #777;
  border-bottom: 1px solid #eee;
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

.format-row {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 6px 12px;
  border-top: 1px solid #eee;
}

.fmt-btn {
  width: 24px;
  height: 24px;
  border: 1px solid #ccc;
  border-radius: 3px;
  background: #f7f7f7;
  font-size: 11px;
  font-weight: 700;
  color: #666;
  flex-shrink: 0;
}

.fmt-btn.active {
  border-color: #4a86e8;
  background: #e8f0fe;
  color: #0a3d91;
}

.fmt-btn.swatch {
  background: linear-gradient(135deg, #fff, #eee);
  background-image:
    linear-gradient(45deg, #ccc 25%, transparent 25%),
    linear-gradient(-45deg, #ccc 25%, transparent 25%),
    linear-gradient(45deg, transparent 75%, #ccc 75%),
    linear-gradient(-45deg, transparent 75%, #ccc 75%);
  background-size: 8px 8px;
  background-position: 0 0, 0 4px, 4px -4px, -4px 0;
}

.fmt-btn.swatch.active {
  background-image: none;
  border-color: #333;
}

.color-input {
  width: 24px;
  height: 24px;
  padding: 0;
  border: 1px solid #ccc;
  border-radius: 3px;
  flex-shrink: 0;
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
