<script setup lang="ts">
import { computed, onMounted } from "vue";
import { storeToRefs } from "pinia";
import { useProvidersStore } from "../stores/providers";
import { SUPPORTED_TARGET_LANGUAGES, useSettingsStore } from "../stores/settings";

const providersStore = useProvidersStore();
const settingsStore = useSettingsStore();
const {
  defaultProviderId,
  translateProviderId,
  targetLanguage,
  shortcut,
  ocrProvider,
  ocrTesseractPath,
  ocrHttpUrl,
  saveMessage,
  saveError,
  saving,
} = storeToRefs(settingsStore);

const providerOptions = computed(() =>
  providersStore.providers.map((provider) => ({
    id: provider.id,
    label: `${provider.name}（${provider.id}）`,
  })),
);

onMounted(() => {
  void settingsStore.load();
});
</script>

<template>
  <section class="card">
    <h2>路由与快捷键</h2>
    <p class="muted">
      不同任务可以走不同 Provider；未单独配置的任务使用默认 Provider。
    </p>
    <p
      v-if="settingsStore.loadError"
      class="error-banner"
    >
      {{ settingsStore.loadError }}
    </p>

    <form
      class="settings-form"
      @submit.prevent="settingsStore.save()"
    >
      <div class="field-row">
        <label class="field">
          <span>默认 Provider</span>
          <select v-model="defaultProviderId">
            <option value="">
              未设置
            </option>
            <option
              v-for="option in providerOptions"
              :key="option.id"
              :value="option.id"
            >
              {{ option.label }}
            </option>
          </select>
        </label>

        <label class="field">
          <span>翻译任务 Provider</span>
          <select v-model="translateProviderId">
            <option value="">
              跟随默认
            </option>
            <option
              v-for="option in providerOptions"
              :key="option.id"
              :value="option.id"
            >
              {{ option.label }}
            </option>
          </select>
        </label>
      </div>

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

        <label class="field">
          <span>OCR 引擎</span>
          <select v-model="ocrProvider">
            <option value="tesseract">
              本机 tesseract CLI
            </option>
            <option value="http">
              本地 OCR HTTP 服务
            </option>
          </select>
        </label>
      </div>

      <label
        v-if="ocrProvider === 'tesseract'"
        class="field"
      >
        <span>tesseract 路径（brew install tesseract）</span>
        <input
          v-model="ocrTesseractPath"
          type="text"
          placeholder="tesseract"
          spellcheck="false"
        >
      </label>
      <label
        v-else
        class="field"
      >
        <span>OCR 服务地址（PaddleOCR sidecar 契约）</span>
        <input
          v-model="ocrHttpUrl"
          type="text"
          placeholder="http://127.0.0.1:8433/ocr"
          spellcheck="false"
        >
      </label>

      <div class="form-footer">
        <button
          class="btn"
          type="submit"
          :disabled="saving"
        >
          {{ saving ? "保存中…" : "保存设置" }}
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
</template>
