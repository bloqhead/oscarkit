<script setup lang="ts">
defineProps<{ title: string; showBack?: boolean }>();
const emit = defineEmits<{ back: [] }>();
</script>

<template>
  <div class="title-bar">
    <button v-if="showBack" class="back-btn" @click="emit('back')" aria-label="Back">‹</button>
    <slot name="leading" />
    <span class="title">{{ title }}</span>
    <div class="spacer" />
    <slot name="trailing" />
  </div>
</template>

<style scoped>
.title-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 9px 12px;
  background: var(--titlebar-grad);
  border-bottom: 3px solid var(--titlebar-border);
  box-shadow: var(--titlebar-shadow);
  color: #fff;
  font-family: var(--font-aim);
}

.title {
  font-weight: 700;
  font-size: 15px;
}

.spacer {
  flex: 1;
}

.back-btn {
  background: none;
  border: none;
  color: #fff;
  font-size: 18px;
  font-weight: 700;
  padding: 0;
  line-height: 1;
}
</style>
