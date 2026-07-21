<script setup lang="ts">
import { useSession } from '../composables/useSession';
import TitleBar from '../components/TitleBar.vue';

const { snapshot, goToBuddyList, soundPrefs, toggleBlock } = useSession();
</script>

<template>
  <div class="prefs-screen">
    <TitleBar title="Preferences" :show-back="true" @back="goToBuddyList" />

    <div class="scroll">
      <section>
        <h2>Account</h2>
        <div class="row">
          <span class="label">Screen name</span>
          <span class="value">{{ snapshot?.screen_name }}</span>
        </div>
      </section>

      <section>
        <h2>Sounds</h2>
        <label class="toggle-row">
          <span>Welcome sound</span>
          <input v-model="soundPrefs.welcome" type="checkbox" />
        </label>
        <label class="toggle-row">
          <span>Goodbye sound</span>
          <input v-model="soundPrefs.goodbye" type="checkbox" />
        </label>
        <label class="toggle-row">
          <span>Buddy sign-on</span>
          <input v-model="soundPrefs.buddySignOn" type="checkbox" />
        </label>
        <label class="toggle-row">
          <span>Buddy sign-off</span>
          <input v-model="soundPrefs.buddySignOff" type="checkbox" />
        </label>
        <label class="toggle-row">
          <span>IM received</span>
          <input v-model="soundPrefs.imReceived" type="checkbox" />
        </label>
        <label class="toggle-row">
          <span>IM sent</span>
          <input v-model="soundPrefs.imSent" type="checkbox" />
        </label>
        <label class="toggle-row">
          <span>Idle reminder</span>
          <input v-model="soundPrefs.idleReminder" type="checkbox" />
        </label>
      </section>

      <section>
        <h2>Privacy / Block List</h2>
        <label v-for="buddy in snapshot?.buddies ?? []" :key="buddy.screen_name" class="toggle-row">
          <span>{{ buddy.screen_name }}</span>
          <input type="checkbox" :checked="buddy.is_blocked" @change="toggleBlock(buddy)" />
        </label>
        <p v-if="!snapshot?.buddies.length" class="empty">No buddies yet.</p>
      </section>

      <section>
        <h2>About</h2>
        <p class="about">OSCARKit 0.1.0 — an OSCAR/AIM client built with Tauri.</p>
      </section>
    </div>
  </div>
</template>

<style scoped>
.prefs-screen {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: #fff;
}

.scroll {
  flex: 1;
  overflow-y: auto;
  padding: 8px 12px;
}

section {
  margin-bottom: 16px;
}

h2 {
  font-size: 12px;
  text-transform: uppercase;
  color: #888;
  margin: 0 0 6px;
  font-family: var(--font-aim);
}

.row {
  display: flex;
  justify-content: space-between;
  font-size: 13px;
  padding: 6px 0;
}

.toggle-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 13px;
  padding: 6px 0;
  border-bottom: 1px solid #f0f0f0;
}

.empty {
  font-size: 12px;
  color: #999;
}

.about {
  font-size: 12px;
  color: #777;
}
</style>
