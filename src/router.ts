import { createRouter, createWebHashHistory } from "vue-router";
import HomeView from "./views/HomeView.vue";
import ProvidersView from "./views/ProvidersView.vue";
import DictionaryView from "./views/DictionaryView.vue";
import VocabularyView from "./views/VocabularyView.vue";
import SettingsView from "./views/SettingsView.vue";

// Tauri 内运行必须用 hash history（file 协议下 path history 无法工作）。
export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", name: "home", component: HomeView },
    { path: "/providers", name: "providers", component: ProvidersView },
    { path: "/dictionary", name: "dictionary", component: DictionaryView },
    { path: "/vocabulary", name: "vocabulary", component: VocabularyView },
    { path: "/settings", name: "settings", component: SettingsView },
  ],
});
