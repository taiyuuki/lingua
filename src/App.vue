<script setup lang="ts">
import { computed, onMounted } from "vue";
import { darkTheme, dateZhCN, zhCN, type GlobalThemeOverrides } from "naive-ui";
import { storeToRefs } from "pinia";
import { useSettingsStore } from "./stores/settings";

// 应用壳：侧边栏导航 + 路由视图；Naive 主题跟随设置（light/dark）。
const settings = useSettingsStore();
const { uiTheme } = storeToRefs(settings);

const naiveTheme = computed(() => (uiTheme.value === "dark" ? darkTheme : null));
const themeOverrides = computed<GlobalThemeOverrides>(() =>
  uiTheme.value === "dark"
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
  { path: "/vocabulary", label: "生词本", icon: "M12 3l2.7 5.6 6.1.8-4.5 4.2 1.1 6-5.4-3-5.4 3 1.1-6L3.2 9.4l6.1-.8z" },
  { path: "/settings", label: "设置", icon: "M4 7h16M4 12h16M4 17h16" },
];

onMounted(() => {
  // 尽早加载设置：Naive 主题与 data-theme（Popup/OCR 样式令牌）都依赖它。
  void settings.load();
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
