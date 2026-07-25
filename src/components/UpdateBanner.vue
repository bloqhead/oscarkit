<script setup lang="ts">
import { useUpdater } from '../composables/useUpdater';

const { updateInfo, isInstalling, installError, installUpdate, dismissUpdate } = useUpdater();
</script>

<template>
  <div v-if="updateInfo" class="update-banner">
    <span class="text">
      {{ isInstalling ? 'Installing update…' : `Update available — v${updateInfo.version}` }}
    </span>
    <p v-if="installError" class="error">{{ installError }}</p>
    <div class="actions">
      <button class="btn-gold" type="button" :disabled="isInstalling" @click="installUpdate">
        {{ isInstalling ? '…' : 'Install & Restart' }}
      </button>
      <button
        class="dismiss"
        type="button"
        :disabled="isInstalling"
        aria-label="Dismiss"
        @click="dismissUpdate"
      >
        ×
      </button>
    </div>
  </div>
</template>

<style scoped>
.update-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 12px;
  background: var(--away-banner-bg);
  border-bottom: 1px solid var(--away-banner-border);
  font-family: var(--font-aim);
  font-size: 12px;
  color: var(--away-banner-text);
  flex-shrink: 0;
}

.text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.error {
  margin: 0;
  font-size: 11px;
  color: var(--badge-red);
}

.actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.dismiss {
  background: none;
  border: none;
  color: var(--away-banner-text);
  font-size: 15px;
  line-height: 1;
  padding: 0 4px;
}
</style>
