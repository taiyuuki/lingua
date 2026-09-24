import { ref } from "vue";
import { defineStore } from "pinia";
import {
  getProviderConfig,
  saveProviderConfig,
  selectionPermissionStatus,
  type ProviderConfigDto,
} from "../lib/api";
import { describeError } from "../lib/errors";

export const SUPPORTED_TARGET_LANGUAGES: Array<{ code: string; label: string }> = [
  { code: "zh", label: "中文" },
  { code: "en", label: "English" },
  { code: "ja", label: "日本語" },
  { code: "ko", label: "한국어" },
  { code: "fr", label: "Français" },
  { code: "de", label: "Deutsch" },
  { code: "es", label: "Español" },
  { code: "ru", label: "Русский" },
];

/**
 * 设置域状态：默认 Provider 表单 + 快捷键 + 权限状态。
 *
 * 注意 store 只做 state + 调 `lib/api`，不包含任何业务逻辑。
 * `apiKeyInput` 为空串表示「不修改已保存的密钥」。
 */
export const useSettingsStore = defineStore("settings", () => {
  const loaded = ref(false);
  const saving = ref(false);
  const loadError = ref<string | null>(null);
  const saveMessage = ref<string | null>(null);
  const saveError = ref<string | null>(null);
  const permissionGranted = ref<boolean | null>(null);

  // 表单字段（可编辑副本）
  const id = ref("default");
  const name = ref("默认 Provider");
  const baseUrl = ref("");
  const model = ref("");
  const targetLanguage = ref("zh");
  const shortcut = ref("alt+t");
  const apiKeyInput = ref("");
  const hasApiKey = ref(false);
  const savedOnce = ref(false);

  async function load(): Promise<void> {
    loadError.value = null;
    try {
      const config = await getProviderConfig();
      if (config) {
        applyConfig(config);
      } else {
        // 首次使用：给出常见本地服务的示例配置起点。
        baseUrl.value = "http://localhost:11434/v1";
        name.value = "默认 Provider";
        id.value = "default";
      }
      permissionGranted.value = await selectionPermissionStatus();
      loaded.value = true;
    } catch (error) {
      loadError.value = describeError(error);
    }
  }

  function applyConfig(config: ProviderConfigDto): void {
    id.value = config.id;
    name.value = config.name;
    baseUrl.value = config.baseUrl;
    model.value = config.model ?? "";
    targetLanguage.value = config.targetLanguage;
    shortcut.value = config.shortcut;
    hasApiKey.value = config.hasApiKey;
    apiKeyInput.value = ""; // 密钥永不出后端，输入框始终清空
    savedOnce.value = true;
  }

  async function save(): Promise<void> {
    saving.value = true;
    saveError.value = null;
    saveMessage.value = null;
    try {
      await saveProviderConfig({
        id: id.value.trim() || "default",
        name: name.value.trim() || "默认 Provider",
        kind: "openai_compatible",
        baseUrl: baseUrl.value.trim(),
        model: model.value.trim() || null,
        organization: null,
        timeoutSecs: 30,
        targetLanguage: targetLanguage.value,
        shortcut: shortcut.value.trim() || "alt+t",
        apiKey: apiKeyInput.value === "" ? null : apiKeyInput.value,
        hasApiKey: hasApiKey.value,
      });
      hasApiKey.value = hasApiKey.value || apiKeyInput.value !== "";
      apiKeyInput.value = "";
      savedOnce.value = true;
      saveMessage.value = "已保存";
    } catch (error) {
      saveError.value = describeError(error);
    } finally {
      saving.value = false;
    }
  }

  return {
    loaded,
    saving,
    loadError,
    saveError,
    saveMessage,
    permissionGranted,
    id,
    name,
    baseUrl,
    model,
    targetLanguage,
    shortcut,
    apiKeyInput,
    hasApiKey,
    savedOnce,
    load,
    save,
  };
});
