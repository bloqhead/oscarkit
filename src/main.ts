import { createApp } from "vue";
import "./styles/base.css";
import App from "./App.vue";
import ImWindow from "./screens/ImWindow.vue";
import ChatWindow from "./screens/ChatWindow.vue";

// Every window (hub, a per-conversation IM window, or a per-room chat
// window) loads this same bundle — a hash in the URL is how a
// freshly-booted window's JS tells them apart. No vue-router: this is a
// one-time branch at boot, not in-app navigation.
const imMatch = location.hash.match(/^#\/im\/(.+)$/);
const chatMatch = location.hash.match(/^#\/chat\/(.+)$/);

const app = imMatch
  ? createApp(ImWindow, { buddyName: decodeURIComponent(imMatch[1]) })
  : chatMatch
    ? createApp(ChatWindow, { roomLabel: decodeURIComponent(chatMatch[1]) })
    : createApp(App);

app.mount("#app");
