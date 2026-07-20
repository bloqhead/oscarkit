<script setup lang="ts">
import { useSession } from '../composables/useSession';
import type { Toast } from '../types';

const { toasts, dismissToast } = useSession();

const ICONS: Record<Toast['kind'], string> = {
  arrive: '🚪',
  depart: '🚪',
  message: '✉',
  error: '⚠',
};
</script>

<template>
  <div class="toast-container">
    <TransitionGroup name="toast">
      <div v-for="toast in toasts" :key="toast.id" class="toast" :class="toast.kind" @click="dismissToast(toast.id)">
        <span class="icon">{{ ICONS[toast.kind] }}</span>
        <span class="text">{{ toast.text }}</span>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
.toast-container {
  position: absolute;
  top: 8px;
  left: 0;
  right: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  pointer-events: none;
  z-index: 100;
}

.toast {
  pointer-events: auto;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border-radius: 6px;
  font-family: var(--font-aim);
  font-size: 12px;
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.3);
  cursor: pointer;
}

.toast.arrive {
  background: #e3f6e8;
  color: #1e6b34;
  border: 1px solid var(--color-online);
}

.toast.depart {
  background: #fdecea;
  color: #9a3412;
  border: 1px solid #e8a23a;
}

.toast.message {
  background: #e8f0fe;
  color: #0a3d91;
  border: 1px solid #4a86e8;
}

.toast.error {
  background: #fdecea;
  color: var(--badge-red);
  border: 1px solid var(--badge-red);
}

.toast-enter-active,
.toast-leave-active {
  transition: all 0.25s ease;
}

.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(-8px);
}
</style>
