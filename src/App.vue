<script setup lang="ts">
// Placeholder UI — proves the Tauri command/event bridge works end to end
// against a real OSCAR session. Not the final design; the real 6-screen
// retro-AIM UI is a separate follow-up pass.
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface Buddy {
  screen_name: string;
  group_name: string;
  is_online: boolean;
  is_away: boolean;
  away_message: string | null;
  warning_level: number;
  is_blocked: boolean;
}

interface IncomingIm {
  from: string;
  text: string;
}

interface SessionSnapshot {
  screen_name: string;
  buddies: Buddy[];
  incoming_messages: IncomingIm[];
  away_message: string | null;
}

const server = ref("");
const screenName = ref("");
const password = ref("");
const loggedIn = ref(false);
const errorMessage = ref("");

const snapshot = ref<SessionSnapshot | null>(null);

const recipient = ref("");
const messageText = ref("");
const awayText = ref("");

listen<SessionSnapshot>("session-update", (event) => {
  snapshot.value = event.payload;
});
listen<string>("session-error", (event) => {
  errorMessage.value = event.payload;
});

async function login() {
  errorMessage.value = "";
  try {
    snapshot.value = await invoke<SessionSnapshot>("login", {
      server: server.value,
      screenName: screenName.value,
      password: password.value,
    });
    loggedIn.value = true;
  } catch (e) {
    errorMessage.value = String(e);
  }
}

async function sendMessage() {
  try {
    await invoke("send_message", { recipient: recipient.value, text: messageText.value });
    messageText.value = "";
  } catch (e) {
    errorMessage.value = String(e);
  }
}

async function setAway() {
  try {
    await invoke("set_away_message", { text: awayText.value || null });
  } catch (e) {
    errorMessage.value = String(e);
  }
}

async function removeBuddy(name: string) {
  try {
    await invoke("remove_buddy", { screenName: name });
  } catch (e) {
    errorMessage.value = String(e);
  }
}

async function requestUserInfo(name: string) {
  try {
    await invoke("request_user_info", { screenName: name });
  } catch (e) {
    errorMessage.value = String(e);
  }
}

async function sendWarning(name: string) {
  try {
    await invoke("send_warning", { screenName: name, anonymous: false });
  } catch (e) {
    errorMessage.value = String(e);
  }
}

async function toggleBlock(buddy: Buddy) {
  try {
    if (buddy.is_blocked) {
      await invoke("remove_from_block_list", { screenName: buddy.screen_name });
    } else {
      await invoke("add_to_block_list", { screenName: buddy.screen_name });
    }
  } catch (e) {
    errorMessage.value = String(e);
  }
}
</script>

<template>
  <main class="container">
    <h1>OSCARKit (placeholder UI)</h1>

    <p v-if="errorMessage" class="error">{{ errorMessage }}</p>

    <form v-if="!loggedIn" @submit.prevent="login">
      <div><input v-model="server" placeholder="Server address (host:port)" /></div>
      <div><input v-model="screenName" placeholder="Screen name" /></div>
      <div><input v-model="password" type="password" placeholder="Password" /></div>
      <button type="submit">Sign On</button>
    </form>

    <div v-else-if="snapshot">
      <h2>Signed on as {{ snapshot.screen_name }}</h2>

      <section>
        <h3>Away message</h3>
        <input v-model="awayText" placeholder="Away message (empty = available)" />
        <button @click="setAway">Set</button>
        <span v-if="snapshot.away_message"> — currently away: {{ snapshot.away_message }}</span>
      </section>

      <section>
        <h3>Buddies</h3>
        <ul>
          <li v-for="buddy in snapshot.buddies" :key="buddy.screen_name">
            <strong>{{ buddy.screen_name }}</strong>
            ({{ buddy.group_name }}) —
            {{ buddy.is_online ? (buddy.is_away ? "away" : "online") : "offline" }}
            <span v-if="buddy.away_message"> "{{ buddy.away_message }}"</span>
            — warning: {{ (buddy.warning_level / 10).toFixed(1) }}%
            <span v-if="buddy.is_blocked"> [blocked]</span>
            <button @click="requestUserInfo(buddy.screen_name)">Info</button>
            <button @click="sendWarning(buddy.screen_name)">Warn</button>
            <button @click="toggleBlock(buddy)">{{ buddy.is_blocked ? "Unblock" : "Block" }}</button>
            <button @click="removeBuddy(buddy.screen_name)">Remove</button>
          </li>
        </ul>
      </section>

      <section>
        <h3>Send a message</h3>
        <input v-model="recipient" placeholder="Recipient" />
        <input v-model="messageText" placeholder="Message" @keydown.enter="sendMessage" />
        <button @click="sendMessage">Send</button>
      </section>

      <section>
        <h3>Incoming messages</h3>
        <ul>
          <li v-for="(im, i) in snapshot.incoming_messages" :key="i">
            <strong>{{ im.from }}:</strong> {{ im.text }}
          </li>
        </ul>
      </section>
    </div>
  </main>
</template>

<style scoped>
.container {
  margin: 0 auto;
  max-width: 640px;
  padding: 2em 1em;
  font-family: sans-serif;
}

.error {
  color: #c62828;
}

section {
  margin-top: 1.5em;
}

input {
  margin-right: 0.4em;
}
</style>
