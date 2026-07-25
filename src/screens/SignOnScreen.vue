<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { useSession } from '../composables/useSession';
import { useCredentials } from '../composables/useCredentials';
import { unlockAudio } from '../utils/sound';

const { login, errorMessage } = useSession();
const {
  savedServer,
  savedScreenName,
  loadSavedCredentials,
  saveCredentials,
  getSavedPassword,
  savePassword,
  deleteSavedPassword,
} = useCredentials();

const server = ref('');
const screenName = ref('');
const password = ref('');
const rememberPassword = ref(false);
const isSubmitting = ref(false);

onMounted(async () => {
  await loadSavedCredentials();
  server.value = savedServer.value;
  screenName.value = savedScreenName.value;

  if (screenName.value) {
    const saved = await getSavedPassword(screenName.value);
    if (saved !== null) {
      password.value = saved;
      rememberPassword.value = true;
    }
  }
});

async function handleSubmit(): Promise<void> {
  // Must happen synchronously, inside this real user-gesture call stack —
  // see unlockAudio()'s doc comment for why.
  unlockAudio();
  isSubmitting.value = true;
  try {
    await login(server.value, screenName.value, password.value);
    await saveCredentials(server.value, screenName.value);
    if (rememberPassword.value) {
      await savePassword(screenName.value, password.value);
    } else {
      await deleteSavedPassword(screenName.value);
    }
  } catch {
    // errorMessage is already set by the composable — nothing else to do.
  } finally {
    isSubmitting.value = false;
  }
}
</script>

<template>
  <div class="signon">
    <div class="mark">
      <svg viewBox="0 0 64 64" width="56" height="56">
        <defs>
          <radialGradient id="markGrad" cx="35%" cy="30%" r="75%">
            <stop offset="0%" stop-color="#fff3b0" />
            <stop offset="100%" stop-color="#e8a800" />
          </radialGradient>
        </defs>
        <rect x="2" y="2" width="60" height="60" rx="14" fill="url(#markGrad)" />
        <circle cx="32" cy="24" r="9" fill="#0a3d91" />
        <path d="M18 50 Q32 30 46 50 Z" fill="#0a3d91" />
      </svg>
    </div>
    <h1 class="wordmark">OSCAR</h1>
    <p class="subtitle">Instant Messenger</p>

    <form class="signon-form" @submit.prevent="handleSubmit">
      <input v-model="server" class="text-input" type="text" placeholder="Server address" autocomplete="off" />
      <input v-model="screenName" class="text-input" type="text" placeholder="Screen name" autocomplete="username" />
      <input v-model="password" class="text-input" type="password" placeholder="Password" autocomplete="current-password" />
      <label class="save-password">
        <input v-model="rememberPassword" type="checkbox" />
        Save password
      </label>
      <p v-if="errorMessage" class="error">{{ errorMessage }}</p>
      <button class="btn-gold sign-on-btn" type="submit" :disabled="isSubmitting">
        {{ isSubmitting ? 'Signing On…' : 'Sign On' }}
      </button>
    </form>
  </div>
</template>

<style scoped>
.signon {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  padding: 24px;
  background: var(--signon-bg);
}

.wordmark {
  margin: 8px 0 0;
  font-size: 26px;
  font-weight: 700;
  color: #0a3d91;
  font-family: var(--font-aim);
}

.subtitle {
  margin: 0 0 20px;
  font-size: 12px;
  color: #4f6a90;
  font-family: var(--font-aim);
}

.signon-form {
  width: 100%;
  max-width: 260px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.save-password {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: #4f6a90;
  font-family: var(--font-aim);
}

.error {
  color: var(--badge-red);
  font-size: 12px;
  margin: 0;
}

.sign-on-btn {
  margin-top: 4px;
  padding: 10px;
  font-size: 15px;
}
</style>
