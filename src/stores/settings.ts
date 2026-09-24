import { ref } from "vue";
import { defineStore } from "pinia";
import {
  broadcastThemeChanged,
  getAppSettings,
  saveAppSettings,
  selectionPermissionStatus,
  type AppSettingsDto,
} from "../lib/api";
import { describeError } from "../lib/errors";
import { applyTheme, normalizeTheme } from "../lib/theme";

/**
 * 应用设置域状态：AI Router 路由（默认 / 翻译任务）+ 目标语言 + 全局快捷键。
 */
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

export const useSettingsStore = defineStore("settings", () => {
  const loaded = ref(false);
  const loadError = ref<string | null>(null);
  const saveMessage = ref<string | null>(null);
  const saveError = ref<string | null>(null);
  const saving = ref(false);
  const permissionGranted = ref<boolean | null>(null);

  // 表单副本
  const defaultProviderId = ref<string>("");
  const translateProviderId = ref<string>(""); // "" = 跟随默认
  const visionProviderId = ref<string>(""); // "" = 自动挑选任一视觉能力 Provider
  const targetLanguage = ref("zh");
  const shortcut = ref("alt+t");
  const ocrShortcut = ref("alt+s");
  const uiTheme = ref("dark");
  const ocrProvider = ref("tesseract");
  const ocrTesseractPath = ref("tesseract");
  const ocrHttpUrl = ref("http://127.0.0.1:8433/ocr");

  async function load(): Promise<void> {
    loadError.value = null;
    try {
      const settings: AppSettingsDto = await getAppSettings();
      defaultProviderId.value = settings.defaultProviderId ?? "";
      translateProviderId.value = settings.translateProviderId ?? "";
      visionProviderId.value = settings.visionProviderId ?? "";
      targetLanguage.value = settings.targetLanguage;
      shortcut.value = settings.shortcut;
      ocrShortcut.value = settings.ocrShortcut;
      uiTheme.value = normalizeTheme(settings.uiTheme);
      applyTheme(uiTheme.value);
      ocrProvider.value = settings.ocrProvider;
      ocrTesseractPath.value = settings.ocrTesseractPath;
      ocrHttpUrl.value = settings.ocrHttpUrl;
      loaded.value = true;
    } catch (error) {
      loadError.value = describeError(error);
    }
    try {
      permissionGranted.value = await selectionPermissionStatus();
    } catch {
      permissionGranted.value = null;
    }
  }

  async function save(): Promise<void> {
    saving.value = true;
    saveError.value = null;
    saveMessage.value = null;
    try {
      await saveAppSettings({
        defaultProviderId: defaultProviderId.value === "" ? null : defaultProviderId.value,
        translateProviderId: translateProviderId.value === "" ? null : translateProviderId.value,
        visionProviderId: visionProviderId.value === "" ? null : visionProviderId.value,
        targetLanguage: targetLanguage.value,
        shortcut: shortcut.value.trim() || "alt+t",
        ocrShortcut: ocrShortcut.value.trim() || "alt+s",
        uiTheme: uiTheme.value,
        ocrProvider: ocrProvider.value,
        ocrTesseractPath: ocrTesseractPath.value.trim() || "tesseract",
        ocrHttpUrl: ocrHttpUrl.value.trim(),
      });
      saveMessage.value = "已保存";
      applyTheme(uiTheme.value);
      // 主窗口应用后广播，Popup 等其它窗口同步切换。
      await broadcastThemeChanged(uiTheme.value);
    } catch (error) {
      saveError.value = describeError(error);
    } finally {
      saving.value = false;
    }
  }

  return {
    loaded,
    loadError,
    saveError,
    saveMessage,
    saving,
    permissionGranted,
    defaultProviderId,
    translateProviderId,
    visionProviderId,
    targetLanguage,
    shortcut,
    ocrShortcut,
    uiTheme,
    ocrProvider,
    ocrTesseractPath,
    ocrHttpUrl,
    load,
    save,
  };
});
