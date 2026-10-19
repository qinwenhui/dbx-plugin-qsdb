import { createApp } from "vue";
import App from "./App.vue";
import "./styles.css";

// ready 之后再挂载：context/locale/theme 在 app 启动时就要用。
const api = window.dbxPlugin;
const boot = api ? api.ready.catch(() => undefined) : Promise.resolve(undefined);
boot.finally(() => {
  createApp(App).mount("#app");
});
