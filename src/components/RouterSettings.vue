<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
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
  NSwitch,
} from "naive-ui";
import { useProvidersStore } from "../stores/providers";
import { SUPPORTED_TARGET_LANGUAGES, useSettingsStore } from "../stores/settings";
import { autostartEnabled, setAutostart } from "../lib/api";

const providersStore = useProvidersStore();
const settingsStore = useSettingsStore();
const {
  defaultProviderId,
  translateProviderId,
  visionProviderId,
  studyProviderId,
  targetLanguage,
  swapLanguage,
  promptTranslateSystem,
  promptTranslateStream,
  shortcut,
  ocrShortcut,
  uiTheme,
  ocrProvider,
  ocrTesseractPath,
  ocrHttpUrl,
  autoVocabulary,
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

// Vision 任务选项：标注哪些 Provider 开启了视觉能力（路由只对
// 有能力的 Provider 生效，其余会被自动改派）。
const visionProviderOptions = computed(() =>
  providersStore.providers.map((provider) => ({
    label: `${provider.name}（${provider.id}）${provider.canVision ? " · 视觉 ✓" : ""}`,
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

// 同语言换向目标："" = 关闭；与目标语言相同由后端视为不换向。
const swapLanguageOptions = [
  { label: "关闭", value: "" },
  ...SUPPORTED_TARGET_LANGUAGES.map((lang) => ({
    label: `${lang.label}（${lang.code}）`,
    value: lang.code,
  })),
];

const ocrProviderOptions = [
  { label: 'Tesseract（本机）', value: 'tesseract' },
  { label: '本地 HTTP 服务', value: 'http' },
];

// Prompt 变量提示（模板里不能直接写字面 {{...}}，会被解析成插值）。
const PROMPT_VARS = ["target_language", "source_language", "mode", "context", "same_language_rule"]
  .map((name) => `{{${name}}}`)
  .join(" ");

onMounted(() => {
  void settingsStore.load();
  void providersStore.load();
});

// 开机自启由 autostart 插件管理（不落 settings 表），本地状态即可；
// 托盘菜单的勾选项与这里双向同步。
const autostart = ref(false);

void autostartEnabled()
  .then((enabled) => {
    autostart.value = enabled;
  })
  .catch(() => {
    // 读取失败保持关闭态。
  });

async function onAutostartChange(enabled: boolean): Promise<void> {
  try {
    await setAutostart(enabled);
    autostart.value = enabled;
  } catch {
    autostart.value = !enabled; // 失败回弹。
  }
}
</script>

<template>
  <n-card
    title="设置"
    size="small"
  >
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
      <p class="form-section-title">
        通用
      </p>
      <div class="form-grid">
        <n-form-item label="界面主题">
          <n-select
            v-model:value="uiTheme"
            :options="themeOptions"
          />
        </n-form-item>
        <n-form-item label="目标语言（划词翻译译入语）">
          <n-select
            v-model:value="targetLanguage"
            :options="languageOptions"
          />
        </n-form-item>
        <n-form-item label="同语言换向">
          <n-select
            v-model:value="swapLanguage"
            :options="swapLanguageOptions"
          />
        </n-form-item>
        <n-form-item label="开机自启">
          <n-switch
            :value="autostart"
            @update:value="onAutostartChange"
          />
        </n-form-item>
        <n-form-item label="查词自动记入生词本">
          <n-switch v-model:value="autoVocabulary" />
        </n-form-item>
        <n-form-item label="划词快捷键">
          <n-input
            v-model:value="shortcut"
            placeholder="alt+t"
          />
        </n-form-item>
        <n-form-item label="截图快捷键">
          <n-input
            v-model:value="ocrShortcut"
            placeholder="alt+s（截光标所在屏幕）"
          />
        </n-form-item>
      </div>

      <p class="form-section-title">
        AI 任务分配
      </p>
      <n-text
        depth="3"
        style="display: block; margin-bottom: 10px; font-size: 12px"
      >
        可为不同任务指定不同的服务；未单独设置的任务会使用默认服务。
      </n-text>
      <div class="form-grid">
        <n-form-item label="默认服务">
          <n-select
            v-model:value="defaultProviderId"
            :options="[{ label: '未设置', value: '' }, ...providerOptions]"
            clearable
          />
        </n-form-item>
        <n-form-item label="翻译任务">
          <n-select
            v-model:value="translateProviderId"
            :options="[{ label: '跟随默认', value: '' }, ...providerOptions]"
            clearable
          />
        </n-form-item>
        <n-form-item
          label="视觉识别"
          class="span-2"
        >
          <n-select
            v-model:value="visionProviderId"
            :options="[{ label: '自动', value: '' }, ...visionProviderOptions]"
            clearable
          />
        </n-form-item>
        <n-form-item
          label="学习分析"
          class="span-2"
        >
          <n-select
            v-model:value="studyProviderId"
            :options="[{ label: '跟随默认', value: '' }, ...providerOptions]"
            clearable
          />
        </n-form-item>
      </div>

      <p class="form-section-title">
        OCR
      </p>
      <div class="form-grid">
        <n-form-item label="识别引擎">
          <n-select
            v-model:value="ocrProvider"
            :options="ocrProviderOptions"
          />
        </n-form-item>
        <n-form-item :label="ocrProvider === 'http' ? '服务地址' : 'tesseract 路径'">
          <n-input
            v-if="ocrProvider === 'tesseract'"
            v-model:value="ocrTesseractPath"
            placeholder="tesseract（Windows: winget install UB-Mannheim.TesseractOCR）"
          />
          <n-input
            v-else
            v-model:value="ocrHttpUrl"
            placeholder="http://127.0.0.1:8433/ocr"
          />
        </n-form-item>
      </div>

      <p class="form-section-title">
        翻译 Prompt（高级）
      </p>
      <n-text
        depth="3"
        style="display: block; margin-bottom: 10px; font-size: 12px"
      >
        留空使用内置模板。可用变量：<code>{{ PROMPT_VARS }}</code>（原文由固定
        user 模板携带，无需包含）。JSON 路径要求模型返回结构化结果，流式路径输出纯文本，请勿混用。
      </n-text>
      <n-form-item label="结构化翻译 system（返回 JSON 的非流式路径）">
        <n-input
          v-model:value="promptTranslateSystem"
          type="textarea"
          :rows="4"
          placeholder="留空使用内置模板"
        />
      </n-form-item>
      <n-form-item label="流式翻译 system（打字机效果路径）">
        <n-input
          v-model:value="promptTranslateStream"
          type="textarea"
          :rows="4"
          placeholder="留空使用内置模板"
        />
      </n-form-item>
      <n-space style="margin-bottom: 12px">
        <n-button
          size="small"
          secondary
          :disabled="!promptTranslateSystem && !promptTranslateStream"
          @click="promptTranslateSystem = ''; promptTranslateStream = ''"
        >
          恢复内置模板
        </n-button>
      </n-space>

      <n-space
        align="center"
        style="margin-top: 16px"
      >
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
        style="margin-top: 12px"
      >
        {{ saveError }}
      </n-alert>
    </n-form>
  </n-card>
</template>
