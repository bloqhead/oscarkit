<script setup lang="ts">
import { onMounted } from 'vue';
import { initImWindow } from '../composables/useImWindow';
import WindowControls from '../components/WindowControls.vue';
import ImScreen from './ImScreen.vue';

const props = defineProps<{ buddyName: string }>();

onMounted(() => {
  initImWindow(props.buddyName);
});
</script>

<template>
  <div class="im-window-shell">
    <WindowControls />
    <div class="frame">
      <ImScreen />
    </div>
  </div>
</template>

<style scoped>
.im-window-shell {
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
