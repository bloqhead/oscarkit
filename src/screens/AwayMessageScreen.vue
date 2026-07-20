<script setup lang="ts">
import { ref } from 'vue';
import { useSession } from '../composables/useSession';
import TitleBar from '../components/TitleBar.vue';

const { snapshot, goToBuddyList, setAway } = useSession();

const PRESETS = [
  'Be right back',
  'Away from my desk, back soon',
  'At work — IM me and I will reply when I can',
  'Gone for the day, talk tomorrow!',
];

const text = ref(snapshot.value?.away_message ?? '');

function applyPreset(preset: string): void {
  text.value = preset;
}

async function handleSave(): Promise<void> {
  const trimmed = text.value.trim();
  await setAway(trimmed ? trimmed : null);
  goToBuddyList();
}
</script>

<template>
  <div class="away-screen">
    <TitleBar title="Away Message" :show-back="true" @back="goToBuddyList" />

    <div class="presets">
      <button
        v-for="preset in PRESETS"
        :key="preset"
        type="button"
        class="preset-row"
        :class="{ selected: text === preset }"
        @click="applyPreset(preset)"
      >
        {{ preset }}
      </button>
    </div>

    <textarea v-model="text" class="text-input away-text" rows="4" placeholder="Custom away message…" />

    <div class="save-row">
      <button class="btn-gold" @click="handleSave">Save</button>
    </div>
  </div>
</template>

<style scoped>
.away-screen {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: #fff;
}

.presets {
  display: flex;
  flex-direction: column;
  padding: 8px 12px;
  gap: 6px;
}

.preset-row {
  text-align: left;
  padding: 8px 10px;
  border-radius: 3px;
  border: 1px solid #ddd;
  background: #fafafa;
  font-family: var(--font-aim);
  font-size: 12px;
  color: #333;
}

.preset-row.selected {
  border-color: #4a86e8;
  background: #e8f0fe;
}

.away-text {
  margin: 4px 12px;
  resize: vertical;
}

.save-row {
  margin-top: auto;
  padding: 12px;
}
</style>
