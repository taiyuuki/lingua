<script setup lang="ts">
import { computed, onMounted } from "vue";
import { darkTheme, dateZhCN, zhCN, type GlobalThemeOverrides } from "naive-ui";
import { storeToRefs } from "pinia";
import { useSettingsStore } from "./stores/settings";
import { useUpdaterStore } from "./stores/updater";

// 应用壳：侧边栏导航 + 路由视图；Naive 主题跟随「已应用」主题（light/dark）。
// 绑 appliedTheme 而非表单值 uiTheme：设置页选主题未保存时不换肤，
// 与 data-theme 令牌（load/save 时才切换）保持一致。
const settings = useSettingsStore();
const { appliedTheme } = storeToRefs(settings);

const updater = useUpdaterStore();
const { hasUpdate } = storeToRefs(updater);

const naiveTheme = computed(() => (appliedTheme.value === "dark" ? darkTheme : null));
const themeOverrides = computed<GlobalThemeOverrides>(() =>
  appliedTheme.value === "dark"
    ? {
        common: {
          primaryColor: "#7f97ff",
          primaryColorHover: "#93a5ff",
          primaryColorPressed: "#93a5ff",
        },
      }
    : {
        common: {
          primaryColor: "#4f6ef7",
          primaryColorHover: "#4059e0",
          primaryColorPressed: "#4059e0",
        },
      },
);

const menu = [
  { path: "/", label: "首页", icon: "M3 11l9-8 9 8M5 10v10h5v-6h4v6h5V10" },
  { path: "/providers", label: "服务", icon: "M7 18a4 4 0 1 1 .6-7.9A5.5 5.5 0 0 1 18 9.5a3.5 3.5 0 0 1 0 7H7z" },
  { path: "/dictionary", label: "词典", icon: "M4 5a2 2 0 0 1 2-2h13v16H6a2 2 0 0 0-2 2zm0 14a2 2 0 0 1 2-2h13" },
  { path: "/dictionaries", label: "词典管理", icon: "M12 3l9 5-9 5-9-5zM3 13.5l9 5 9-5" },
  { path: "/vocabulary", label: "生词本", icon: "M12 3l2.7 5.6 6.1.8-4.5 4.2 1.1 6-5.4-3-5.4 3 1.1-6L3.2 9.4l6.1-.8z" },
  { path: "/history", label: "历史", icon: "M12 8v5l3 3M21 12a9 9 0 1 1-9-9 9 9 0 0 1 9 9z" },
  { path: "/settings", label: "设置", icon: "M4 7h16M4 12h16M4 17h16" },
];

onMounted(() => {
  // 尽早加载设置：Naive 主题与 data-theme（Popup/OCR 样式令牌）都依赖它。
  void settings.load();
  // 延迟静默检查更新：不与启动期 IPC 抢通道，失败不打扰用户；
  // 有新版本时「设置」菜单出现红点，入口在设置页的「关于与更新」。
  window.setTimeout(() => void updater.check(true), 3000);
});
</script>

<template>
  <n-config-provider
    :theme="naiveTheme"
    :theme-overrides="themeOverrides"
    :locale="zhCN"
    :date-locale="dateZhCN"
    class="app-provider"
  >
    <div class="app-shell">
      <aside class="app-sidebar">
        <div class="app-brand">
          Lingua
        </div>
        <nav class="app-menu">
          <RouterLink
            v-for="item in menu"
            :key="item.path"
            :to="item.path"
            class="menu-item"
            active-class="active"
          >
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path :d="item.icon" />
            </svg>
            <span>{{ item.label }}</span>
            <span
              v-if="item.path === '/settings' && hasUpdate"
              class="menu-update-dot"
            />
          </RouterLink>
        </nav>
        <div class="app-sidebar-footer">
          <span>AI 翻译与语言学习助手</span>
        </div>
      </aside>
      <main class="app-content">
        <RouterView />
      </main>
    </div>
  </n-config-provider>
</template>
