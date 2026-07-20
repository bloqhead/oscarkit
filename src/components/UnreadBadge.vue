<script setup lang="ts">
defineProps<{ count: number }>();
</script>

<template>
  <span v-if="count > 0" class="unread-badge">{{ count }}</span>
</template>

<style scoped>
.unread-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: 8px;
  background: var(--badge-red);
  color: #fff;
  font-size: 10px;
  font-weight: 700;
  font-family: var(--font-aim);
}
</style>
