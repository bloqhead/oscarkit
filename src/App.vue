<script setup lang="ts">
import { useSession } from './composables/useSession';
import WindowControls from './components/WindowControls.vue';
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
    <WindowControls />
    <div class="frame-wrap">
      <div class="phone-frame">
        <SignOnScreen v-if="currentScreen === 'signon'" />
        <BuddyListScreen v-else-if="currentScreen === 'buddylist'" />
        <ImScreen v-else-if="currentScreen === 'im'" />
        <BuddyInfoScreen v-else-if="currentScreen === 'info'" />
        <AwayMessageScreen v-else-if="currentScreen === 'away'" />
        <PreferencesScreen v-else-if="currentScreen === 'preferences'" />
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
}
</style>
