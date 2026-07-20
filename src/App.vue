<script setup lang="ts">
import { useSession } from './composables/useSession';
import SignOnScreen from './screens/SignOnScreen.vue';
import BuddyListScreen from './screens/BuddyListScreen.vue';
import ImScreen from './screens/ImScreen.vue';
import BuddyInfoScreen from './screens/BuddyInfoScreen.vue';
import AwayMessageScreen from './screens/AwayMessageScreen.vue';
import PreferencesScreen from './screens/PreferencesScreen.vue';

const { currentScreen } = useSession();
</script>

<template>
  <div class="app-shell">
    <div class="phone-frame">
      <SignOnScreen v-if="currentScreen === 'signon'" />
      <BuddyListScreen v-else-if="currentScreen === 'buddylist'" />
      <ImScreen v-else-if="currentScreen === 'im'" />
      <BuddyInfoScreen v-else-if="currentScreen === 'info'" />
      <AwayMessageScreen v-else-if="currentScreen === 'away'" />
      <PreferencesScreen v-else-if="currentScreen === 'preferences'" />
    </div>
  </div>
</template>

<style scoped>
.app-shell {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #2b2b2b;
}

.phone-frame {
  width: 390px;
  height: 100%;
  max-height: 780px;
  background: #fff;
  box-shadow: 0 0 24px rgba(0, 0, 0, 0.5);
  overflow: hidden;
  position: relative;
  border-radius: 8px;
}
</style>
