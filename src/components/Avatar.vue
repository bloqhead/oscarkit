<script setup lang="ts">
import { computed } from 'vue';
import { initials } from '../utils/format';

const props = defineProps<{ name: string; active: boolean }>();

const label = computed(() => initials(props.name));
</script>

<template>
  <div class="avatar" :class="{ offline: !active }">{{ label }}</div>
</template>

<style scoped>
.avatar {
  width: 36px;
  height: 36px;
  border-radius: 50%;
  background: var(--avatar-active-bg);
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  font-family: var(--font-aim);
  font-weight: 700;
  font-size: 13px;
  flex-shrink: 0;
}

.avatar.offline {
  background: var(--avatar-offline-bg);
  opacity: 0.5;
}
</style>
