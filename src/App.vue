<script setup lang="ts">
import { onMounted } from "vue";
import { storeToRefs } from "pinia";
import { useAppStore } from "./stores/app";
import { useSettingsStore, SUPPORTED_TARGET_LANGUAGES } from "./stores/settings";

const appStore = useAppStore();
const { appInfo, database, errorMessage } = storeToRefs(appStore);

const settings = useSettingsStore();
const {
  baseUrl,
  model,
  targetLanguage,
  shortcut,
  apiKeyInput,
  hasApiKey,
  savedOnce,
  permissionGranted,
  saveMessage,
  saveError,
  loadError,
  saving,
} = storeToRefs(settings);

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
        AI 翻译与语言学习助手 · Phase 1：划词翻译链路
      </p>
    </header>

    <section class="card">
      <h2>使用方法</h2>
      <p class="muted">
        在任意应用中选中文本，按全局快捷键
        <code class="kbd">{{ settings.shortcut || "alt+t" }}</code>
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

    <section class="card">
      <h2>AI Provider 设置</h2>
      <p class="muted">
        兼容 OpenAI 协议的服务均可接入（OpenAI / DeepSeek / 本地 Ollama、LM
        Studio 等）。API Key 保存在系统钥匙串中，不会写入数据库。
      </p>
      <p
        v-if="loadError"
        class="error-banner"
      >
        {{ loadError }}
      </p>

      <form
        class="settings-form"
        @submit.prevent="settings.save()"
      >
        <label class="field">
          <span>接口地址 (baseUrl)</span>
          <input
            v-model="baseUrl"
            type="text"
            placeholder="https://api.deepseek.com/v1 或 http://localhost:11434/v1"
            spellcheck="false"
            required
          >
        </label>

        <label class="field">
          <span>模型</span>
          <input
            v-model="model"
            type="text"
            placeholder="deepseek-chat / qwen2.5:7b …"
            spellcheck="false"
          >
        </label>

        <label class="field">
          <span>API Key</span>
          <input
            v-model="apiKeyInput"
            type="password"
            :placeholder="hasApiKey ? '已保存（留空表示不修改）' : 'sk-…'"
            autocomplete="off"
          >
        </label>

        <div class="field-row">
          <label class="field">
            <span>目标语言</span>
            <select v-model="targetLanguage">
              <option
                v-for="lang in SUPPORTED_TARGET_LANGUAGES"
                :key="lang.code"
                :value="lang.code"
              >
                {{ lang.label }}（{{ lang.code }}）
              </option>
            </select>
          </label>

          <label class="field">
            <span>全局快捷键</span>
            <input
              v-model="shortcut"
              type="text"
              placeholder="alt+t"
              spellcheck="false"
            >
          </label>
        </div>

        <div class="form-footer">
          <button
            class="btn"
            type="submit"
            :disabled="saving"
          >
            {{ saving ? "保存中…" : savedOnce ? "保存修改" : "保存配置" }}
          </button>
          <span
            v-if="saveMessage"
            class="save-ok"
          >{{ saveMessage }}</span>
        </div>
        <p
          v-if="saveError"
          class="error-banner"
        >
          {{ saveError }}
        </p>
      </form>
    </section>

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
