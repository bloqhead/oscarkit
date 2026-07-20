<script setup lang="ts">
import { computed, onMounted } from 'vue';
import { useSession } from '../composables/useSession';
import { buddyStatus, formatWarningLevel, statusLabel } from '../utils/format';
import TitleBar from '../components/TitleBar.vue';
import Avatar from '../components/Avatar.vue';
import StatusDot from '../components/StatusDot.vue';

const { infoBuddy, getBuddy, backFromInfo, goToIm, requestInfo, warnBuddy, toggleBlock } = useSession();

const buddy = computed(() => (infoBuddy.value ? getBuddy(infoBuddy.value) : undefined));

async function handleWarn(): Promise<void> {
  if (buddy.value) await warnBuddy(buddy.value.screen_name, true);
}

async function handleToggleBlock(): Promise<void> {
  if (buddy.value) await toggleBlock(buddy.value);
}

onMounted(() => {
  if (infoBuddy.value) requestInfo(infoBuddy.value).catch(() => {});
});
</script>

<template>
  <div v-if="buddy" class="info-screen">
    <TitleBar title="Buddy Info" :show-back="true" @back="backFromInfo" />

    <div class="card">
      <Avatar :name="buddy.screen_name" :active="buddyStatus(buddy) !== 'offline'" />
      <div class="name">{{ buddy.screen_name }}</div>
      <div class="status-label">{{ statusLabel(buddy) }}</div>
      <StatusDot :status="buddyStatus(buddy)" />
    </div>

    <div class="details">
      <div class="detail-row">
        <span class="label">Warning level</span>
        <span class="value">{{ formatWarningLevel(buddy.warning_level) }}</span>
      </div>
    </div>

    <div v-if="buddy.away_message" class="away-box">{{ buddy.away_message }}</div>

    <div class="actions">
      <button class="btn-secondary" @click="goToIm(buddy.screen_name)">IM</button>
      <button class="btn-secondary" @click="handleWarn">Warn</button>
      <button class="btn-secondary" @click="handleToggleBlock">{{ buddy.is_blocked ? 'Unblock' : 'Block' }}</button>
    </div>
  </div>
</template>

<style scoped>
.info-screen {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: #fff;
}

.card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 20px;
  border-bottom: 1px solid #eee;
}

.name {
  font-family: var(--font-aim);
  font-weight: 700;
  color: var(--color-name-online);
  font-size: 15px;
}

.status-label {
  font-style: italic;
  font-size: 12px;
  color: #777;
}

.details {
  padding: 12px;
}

.detail-row {
  display: flex;
  justify-content: space-between;
  font-size: 13px;
  padding: 6px 0;
  border-bottom: 1px solid #f0f0f0;
}

.label {
  color: #777;
}

.away-box {
  margin: 0 12px 12px;
  padding: 10px;
  background: var(--away-banner-bg);
  border: 1px solid var(--away-banner-border);
  color: var(--away-banner-text);
  font-size: 13px;
  border-radius: 4px;
}

.actions {
  display: flex;
  gap: 8px;
  padding: 12px;
  margin-top: auto;
}
</style>
