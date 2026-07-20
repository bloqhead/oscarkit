<script setup lang="ts">
import { getCurrentWindow } from '@tauri-apps/api/window';

const appWindow = getCurrentWindow();

function minimize(): void {
  appWindow.minimize();
}

function close(): void {
  appWindow.close();
}
</script>

<template>
  <div class="window-controls" data-tauri-drag-region>
    <button class="ctrl-btn" type="button" aria-label="Minimize" @click="minimize">–</button>
    <button class="ctrl-btn close" type="button" aria-label="Close" @click="close">×</button>
  </div>
</template>

<style scoped>
.window-controls {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 26px;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  z-index: 200;
  user-select: none;
}

.ctrl-btn {
  width: 32px;
  height: 26px;
  border: none;
  background: transparent;
  color: rgba(255, 255, 255, 0.55);
  font-family: var(--font-aim);
  font-size: 15px;
  line-height: 1;
}

.ctrl-btn:hover {
  background: rgba(255, 255, 255, 0.15);
  color: #fff;
}

.ctrl-btn.close:hover {
  background: #e0403f;
  color: #fff;
}
</style>
