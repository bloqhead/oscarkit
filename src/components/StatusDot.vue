<script setup lang="ts">
import { computed } from 'vue';
import type { BuddyStatus } from '../utils/format';

const props = defineProps<{ status: BuddyStatus }>();

const color = computed(() => {
  if (props.status === 'online') return 'var(--color-online)';
  if (props.status === 'away') return 'var(--color-away)';
  return 'var(--color-offline)';
});
</script>

<template>
  <span class="status-dot" :style="{ backgroundColor: color }" />
</template>

<style scoped>
.status-dot {
  display: inline-block;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  flex-shrink: 0;
  box-shadow: inset 0 1px 1px rgba(0, 0, 0, 0.2);
}
</style>
