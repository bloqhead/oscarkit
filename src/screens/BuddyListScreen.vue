<script setup lang="ts">
import { ref } from 'vue';
import { useSession } from '../composables/useSession';
import { buddyStatus, statusLabel } from '../utils/format';
import TitleBar from '../components/TitleBar.vue';
import Avatar from '../components/Avatar.vue';
import StatusDot from '../components/StatusDot.vue';
import WarningBadge from '../components/WarningBadge.vue';
import BlockedBadge from '../components/BlockedBadge.vue';
import UnreadBadge from '../components/UnreadBadge.vue';
import ToastContainer from '../components/ToastContainer.vue';

const { snapshot, groupedBuddies, unreadFor, goToIm, goToAway, goToPreferences, clearAway, logout, addBuddy } =
  useSession();

async function handleImBack(): Promise<void> {
  await clearAway();
}

async function handleSignOff(): Promise<void> {
  await logout();
}

const showAddBuddy = ref(false);
const newBuddyName = ref('');
const newBuddyGroup = ref('');

async function handleAddBuddy(): Promise<void> {
  const name = newBuddyName.value.trim();
  if (!name) return;
  const group = newBuddyGroup.value.trim() || 'Buddies';
  try {
    await addBuddy(name, group);
    newBuddyName.value = '';
    newBuddyGroup.value = '';
    showAddBuddy.value = false;
  } catch {
    // Failure is already surfaced as an error toast by the composable —
    // leave the form open so the user can fix the input and retry.
  }
}
</script>

<template>
  <div class="buddy-list">
    <ToastContainer />
    <TitleBar title="Buddy List" />

    <div class="action-row">
      <button class="btn-secondary" @click="goToAway">Away</button>
      <button class="btn-secondary" @click="goToPreferences">Setup</button>
      <button class="btn-secondary" @click="handleSignOff">Sign Off</button>
    </div>

    <div v-if="snapshot?.away_message" class="away-banner">
      <span>You are currently Away</span>
      <button class="btn-secondary" @click="handleImBack">I'm Back</button>
    </div>

    <div class="add-buddy-row">
      <button v-if="!showAddBuddy" class="btn-secondary" @click="showAddBuddy = true">Add Buddy</button>
      <form v-else class="add-buddy-form" @submit.prevent="handleAddBuddy">
        <input v-model="newBuddyName" class="text-input" type="text" placeholder="Screen name" autofocus />
        <input v-model="newBuddyGroup" class="text-input" type="text" placeholder="Group (optional)" />
        <button class="btn-gold" type="submit">Add</button>
        <button class="btn-secondary" type="button" @click="showAddBuddy = false">Cancel</button>
      </form>
    </div>

    <div class="groups">
      <div v-for="group in groupedBuddies" :key="group.name" class="group">
        <div class="group-header">{{ group.name.toUpperCase() }} ({{ group.online }}/{{ group.total }})</div>
        <div
          v-for="buddy in group.buddies"
          :key="buddy.screen_name"
          class="buddy-row"
          @click="goToIm(buddy.screen_name)"
        >
          <Avatar :name="buddy.screen_name" :active="buddyStatus(buddy) !== 'offline'" />
          <div class="buddy-main">
            <div
              class="buddy-name"
              :class="{ online: buddyStatus(buddy) === 'online', away: buddyStatus(buddy) === 'away', offline: buddyStatus(buddy) === 'offline' }"
            >
              {{ buddy.screen_name }}
            </div>
            <div class="buddy-status">{{ statusLabel(buddy) }}</div>
          </div>
          <WarningBadge :level="buddy.warning_level" />
          <BlockedBadge v-if="buddy.is_blocked" />
          <UnreadBadge :count="unreadFor(buddy.screen_name)" />
          <StatusDot :status="buddyStatus(buddy)" />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.buddy-list {
  position: relative;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: #fff;
}

.action-row {
  display: flex;
  gap: 8px;
  padding: 8px 12px;
  border-bottom: 1px solid #ddd;
}

.away-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 12px;
  background: var(--away-banner-bg);
  border-bottom: 1px solid var(--away-banner-border);
  color: var(--away-banner-text);
  font-style: italic;
  font-size: 12px;
  font-family: var(--font-aim);
}

.add-buddy-row {
  padding: 8px 12px;
  border-bottom: 1px solid #ddd;
}

.add-buddy-form {
  display: flex;
  gap: 6px;
}

.add-buddy-form .text-input {
  flex: 1;
  min-width: 0;
}

.groups {
  flex: 1;
  overflow-y: auto;
}

.group-header {
  padding: 4px 12px;
  font-size: 11px;
  font-weight: 700;
  color: #555;
  background: linear-gradient(180deg, #f4f4f0, #e6e6df);
}

.buddy-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  cursor: pointer;
  border-bottom: 1px solid #f0f0f0;
}

.buddy-row:hover {
  background: #f5f8fd;
}

.buddy-main {
  flex: 1;
  min-width: 0;
}

.buddy-name {
  font-family: var(--font-aim);
  font-size: 13px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.buddy-name.online {
  font-weight: 700;
  color: var(--color-name-online);
}

.buddy-name.away {
  font-style: italic;
  color: var(--color-name-away);
}

.buddy-name.offline {
  color: var(--color-name-offline);
}

.buddy-status {
  font-size: 11px;
  color: #888;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
