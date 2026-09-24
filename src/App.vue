<script setup lang="ts">
import { onMounted } from "vue";
import { storeToRefs } from "pinia";
import { useAppStore } from "./stores/app";
import { useSettingsStore } from "./stores/settings";
import ProviderManager from "./components/ProviderManager.vue";
import DictionaryManager from "./components/DictionaryManager.vue";
import RouterSettings from "./components/RouterSettings.vue";

const appStore = useAppStore();
const { appInfo, database, errorMessage } = storeToRefs(appStore);

const settings = useSettingsStore();
const { shortcut, permissionGranted } = storeToRefs(settings);

onMounted(() => {
  void appStore.refresh();
  void settings.load();
});
</script>

<template>
  <main class="page">
    <header class="page-header">
      <h1>Lingua</h1>
      <p class="subtitle">
        AI 翻译与语言学习助手 · Phase 2：多 Provider + AI Router
      </p>
    </header>

    <section class="card">
      <h2>使用方法</h2>
      <p class="muted">
        在任意应用中选中文本，按全局快捷键
        <code class="kbd">{{ shortcut || "alt+t" }}</code>
        ，翻译结果会在光标附近的 Popup 中显示。按 <code class="kbd">Esc</code>
        或点击窗口外部可关闭 Popup。
      </p>
      <p
        v-if="permissionGranted === false"
        class="warn-banner"
      >
        缺少辅助功能权限：请在「系统设置 → 隐私与安全性 → 辅助功能」中授权
        Lingua，否则无法读取选中文本。
      </p>
      <dl
        v-if="appInfo"
        class="status-grid"
      >
        <dt>应用</dt>
        <dd>{{ appInfo.name }} v{{ appInfo.version }}（{{ appInfo.os }}/{{ appInfo.arch }}）</dd>
        <dt>数据库</dt>
        <dd v-if="database">
          schema v{{ database.schemaVersion }} · {{ database.migrationsTotal }} 个 migration
        </dd>
      </dl>
      <p
        v-if="errorMessage"
        class="error-banner"
      >
        {{ errorMessage }}
      </p>
    </section>

    <ProviderManager />
    <DictionaryManager />
    <RouterSettings />

    <section class="card">
      <h2>开发</h2>
      <button
        class="btn btn-secondary"
        type="button"
        @click="appStore.openPopupWindow()"
      >
        打开 Popup 窗口
      </button>
    </section>
  </main>
</template>
