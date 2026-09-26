import { createRouter, createWebHashHistory } from "vue-router";
import HomeView from "./views/HomeView.vue";
import ProvidersView from "./views/ProvidersView.vue";
import DictionaryView from "./views/DictionaryView.vue";
import DictionaryListView from "./views/DictionaryListView.vue";
import DictionaryImportView from "./views/DictionaryImportView.vue";
import VocabularyView from "./views/VocabularyView.vue";
import HistoryView from "./views/HistoryView.vue";
import SettingsView from "./views/SettingsView.vue";

// Tauri 内运行必须用 hash history（file 协议下 path history 无法工作）。
export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", name: "home", component: HomeView },
    { path: "/providers", name: "providers", component: ProvidersView },
    { path: "/dictionary", name: "dictionary", component: DictionaryView },
    { path: "/dictionaries", name: "dictionaries", component: DictionaryListView },
    { path: "/dictionaries/import", name: "dictionary-import", component: DictionaryImportView },
    { path: "/vocabulary", name: "vocabulary", component: VocabularyView },
    { path: "/history", name: "history", component: HistoryView },
    { path: "/settings", name: "settings", component: SettingsView },
  ],
});
