import { createApp } from "vue";
import "./styles/base.css";
import App from "./App.vue";
import ImWindow from "./screens/ImWindow.vue";

// Every window (hub or a per-conversation IM window) loads this same bundle
// — a hash in the URL is how a freshly-booted window's JS tells them apart.
// No vue-router: this is a one-time branch at boot, not in-app navigation.
const imMatch = location.hash.match(/^#\/im\/(.+)$/);

const app = imMatch
  ? createApp(ImWindow, { buddyName: decodeURIComponent(imMatch[1]) })
  : createApp(App);

app.mount("#app");
