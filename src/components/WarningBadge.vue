<script setup lang="ts">
import { formatWarningLevel } from '../utils/format';

defineProps<{ level: number }>();
</script>

<template>
  <span v-if="level > 0" class="warning-badge">{{ formatWarningLevel(level) }}</span>
</template>

<style scoped>
.warning-badge {
  display: inline-block;
  background: var(--badge-red);
  color: #fff;
  font-weight: 700;
  font-size: 10px;
  border-radius: 8px;
  padding: 1px 6px;
  font-family: var(--font-aim);
}
</style>
