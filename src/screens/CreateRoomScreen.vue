<script setup lang="ts">
import { ref } from 'vue';
import { useSession } from '../composables/useSession';
import TitleBar from '../components/TitleBar.vue';

const { groupedBuddies, goToBuddyList, createRoom } = useSession();

const roomName = ref('');
const selected = ref(new Set<string>());
const submitting = ref(false);

function toggle(screenName: string): void {
  if (selected.value.has(screenName)) {
    selected.value.delete(screenName);
  } else {
    selected.value.add(screenName);
  }
}

async function handleCreate(): Promise<void> {
  const name = roomName.value.trim();
  if (!name || submitting.value) return;
  submitting.value = true;
  try {
    await createRoom(name, [...selected.value]);
  } catch {
    // Failure already surfaced as an error toast by the composable — leave
    // the form filled in so the user can retry.
    submitting.value = false;
  }
}
</script>

<template>
  <div class="create-room-screen">
    <TitleBar title="Create Room" :show-back="true" @back="goToBuddyList" />

    <form class="create-room-form" @submit.prevent="handleCreate">
      <input v-model="roomName" class="text-input room-name-input" type="text" placeholder="Room name" autofocus />

      <div class="invite-label">Invite:</div>
      <div class="buddy-picker">
        <div v-for="group in groupedBuddies" :key="group.name" class="group">
          <div class="group-header">{{ group.name.toUpperCase() }}</div>
          <label v-for="buddy in group.buddies" :key="buddy.screen_name" class="buddy-check-row">
            <input
              type="checkbox"
              :checked="selected.has(buddy.screen_name)"
              @change="toggle(buddy.screen_name)"
            />
            <span>{{ buddy.screen_name }}</span>
          </label>
        </div>
      </div>

      <div class="submit-row">
        <button class="btn-gold" type="submit" :disabled="!roomName.trim() || submitting">
          {{ submitting ? 'Creating…' : 'Create Room' }}
        </button>
      </div>
    </form>
  </div>
</template>

<style scoped>
.create-room-screen {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: #fff;
}

.create-room-form {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.room-name-input {
  margin: 12px;
}

.invite-label {
  padding: 0 12px;
  font-size: 11px;
  font-weight: 700;
  color: #555;
}

.buddy-picker {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  margin-top: 4px;
}

.group-header {
  padding: 4px 12px;
  font-size: 11px;
  font-weight: 700;
  color: #555;
  background: linear-gradient(180deg, #f4f4f0, #e6e6df);
}

.buddy-check-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  font-size: 13px;
  font-family: var(--font-aim);
  border-bottom: 1px solid #f0f0f0;
}

.submit-row {
  padding: 12px;
}
</style>
