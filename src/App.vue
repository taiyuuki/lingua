<script setup lang="ts">
import { onMounted } from "vue";
import { storeToRefs } from "pinia";
import { useAppStore } from "./stores/app";

const store = useAppStore();
const { appInfo, database, errorMessage, loading } = storeToRefs(store);

onMounted(() => {
  void store.refresh();
});
</script>

<template>
  <main class="page">
    <header class="page-header">
      <h1>Lingua</h1>
      <p class="subtitle">
        AI 翻译与语言学习助手 · Phase 0 架构骨架
      </p>
    </header>

    <section class="card">
      <h2>运行状态</h2>
      <p
        v-if="loading"
        class="muted"
      >
        加载中…
      </p>
      <dl
        v-if="appInfo"
        class="status-grid"
      >
        <dt>应用</dt>
        <dd>{{ appInfo.name }} v{{ appInfo.version }}</dd>
        <dt>平台</dt>
        <dd>{{ appInfo.os }} / {{ appInfo.arch }}</dd>
        <template v-if="database">
          <dt>数据库</dt>
          <dd>schema v{{ database.schemaVersion }} · {{ database.migrationsTotal }} 个 migration</dd>
        </template>
      </dl>
      <p
        v-if="errorMessage"
        class="error-banner"
      >
        {{ errorMessage }}
      </p>
    </section>

    <section class="card">
      <h2>窗口系统</h2>
      <p class="muted">
        Popup 为无边框、置顶、失焦自动隐藏的轻量窗口。
        Phase 1 将由全局快捷键触发，并按选中文本位置定位。
      </p>
      <button
        class="btn"
        type="button"
        @click="store.openPopupWindow()"
      >
        打开 Popup 窗口
      </button>
    </section>
  </main>
</template>
