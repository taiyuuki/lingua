<script setup lang="ts">
import { computed, onMounted } from "vue";
import { storeToRefs } from "pinia";
import {
  NAlert,
  NButton,
  NCard,
  NForm,
  NFormItem,
  NInput,
  NSelect,
  NSpace,
} from "naive-ui";
import { useProvidersStore } from "../stores/providers";
import { SUPPORTED_TARGET_LANGUAGES, useSettingsStore } from "../stores/settings";

const providersStore = useProvidersStore();
const settingsStore = useSettingsStore();
const {
  defaultProviderId,
  translateProviderId,
  targetLanguage,
  shortcut,
  uiTheme,
  ocrProvider,
  ocrTesseractPath,
  ocrHttpUrl,
  saveMessage,
  saveError,
  saving,
} = storeToRefs(settingsStore);

const providerOptions = computed(() =>
  providersStore.providers.map((provider) => ({
    label: `${provider.name}（${provider.id}）`,
    value: provider.id,
  })),
);

const themeOptions = [
  { label: "深色", value: "dark" },
  { label: "浅色", value: "light" },
];

const languageOptions = SUPPORTED_TARGET_LANGUAGES.map((lang) => ({
  label: `${lang.label}（${lang.code}）`,
  value: lang.code,
}));

const ocrProviderOptions = [
  { label: '本机 tesseract CLI', value: 'tesseract' },
  { label: '本地 OCR HTTP 服务', value: 'http' },
];

onMounted(() => {
  void settingsStore.load();
  void providersStore.load();
});
</script>

<template>
  <n-card
    title="设置"
    size="small"
  >
    <n-text
      depth="3"
      style="display: block; margin-bottom: 12px"
    >
      不同任务可以走不同 Provider；未单独配置的任务使用默认 Provider。
    </n-text>
    <n-alert
      v-if="settingsStore.loadError"
      type="error"
      :show-icon="true"
      style="margin-bottom: 12px"
    >
      {{ settingsStore.loadError }}
    </n-alert>

    <n-form
      label-placement="top"
      size="small"
      :show-feedback="false"
    >
      <n-space
        vertical
        :size="12"
      >
        <n-space :size="12">
          <n-form-item
            label="界面主题"
            style="flex: 1"
          >
            <n-select
              v-model:value="uiTheme"
              :options="themeOptions"
            />
          </n-form-item>
          <n-form-item
            label="默认 Provider"
            style="flex: 1"
          >
            <n-select
              v-model:value="defaultProviderId"
              :options="[{ label: '未设置', value: '' }, ...providerOptions]"
              clearable
            />
          </n-form-item>
          <n-form-item
            label="翻译任务 Provider"
            style="flex: 1"
          >
            <n-select
              v-model:value="translateProviderId"
              :options="[{ label: '跟随默认', value: '' }, ...providerOptions]"
              clearable
            />
          </n-form-item>
        </n-space>

        <n-space :size="12">
          <n-form-item
            label="目标语言"
            style="flex: 1"
          >
            <n-select
              v-model:value="targetLanguage"
              :options="languageOptions"
            />
          </n-form-item>
          <n-form-item
            label="全局快捷键"
            style="flex: 1"
          >
            <n-input
              v-model:value="shortcut"
              placeholder="alt+t"
            />
          </n-form-item>
        </n-space>

        <n-space :size="12">
          <n-form-item
            label="OCR 引擎"
            style="flex: 1"
          >
            <n-select
              v-model:value="ocrProvider"
              :options="ocrProviderOptions"
            />
          </n-form-item>
          <n-form-item
            :label="ocrProvider === 'http' ? 'OCR 服务地址（PaddleOCR sidecar 契约）' : 'tesseract 路径（macOS: brew install tesseract；Windows: winget install UB-Mannheim.TesseractOCR）'"
            style="flex: 2"
          >
            <n-input
              v-if="ocrProvider === 'tesseract'"
              v-model:value="ocrTesseractPath"
              placeholder="tesseract"
            />
            <n-input
              v-else
              v-model:value="ocrHttpUrl"
              placeholder="http://127.0.0.1:8433/ocr"
            />
          </n-form-item>
        </n-space>

        <n-space align="center">
          <n-button
            type="primary"
            :loading="saving"
            @click="settingsStore.save()"
          >
            保存设置
          </n-button>
          <n-text
            v-if="saveMessage"
            type="success"
          >
            {{ saveMessage }}
          </n-text>
        </n-space>
        <n-alert
          v-if="saveError"
          type="error"
          :show-icon="true"
        >
          {{ saveError }}
        </n-alert>
      </n-space>
    </n-form>
  </n-card>
</template>
