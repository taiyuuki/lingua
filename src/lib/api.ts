/**
 * 后端命令的类型化封装 —— 前端触达 Rust 的唯一通道。
 *
 * 约定：组件与 store 里不允许直接 `invoke()` / `listen()`，
 * 必须经过这里；命令名、参数与返回结构的契约集中在一个文件里。
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface AppInfo {
  name: string;
  version: string;
  os: string;
  arch: string;
}

export interface DatabaseStatus {
  schemaVersion: number;
  migrationsTotal: number;
}

/** 与 Rust `PopupState` 对应（internally tagged）。 */
export type PopupState =
  | { status: "loading" }
  | ({ status: "result" } & TranslationResult)
  | { status: "error"; kind: string; message: string };

/** 与 Rust `TranslationResult` 对应（camelCase）。 */
export interface TranslationResult {
  sourceText: string;
  translatedText: string;
  detectedLanguage: string | null;
  alternatives: string[];
  explanation: string | null;
}

/**
 * Provider 配置。`apiKey` 永远不会被后端返回（恒为 null）；
 * 保存时：null = 不修改，"" = 删除，非空 = 覆盖。
 */
export interface ProviderConfigDto {
  id: string;
  name: string;
  kind: "openai_compatible" | "openai";
  baseUrl: string;
  model: string | null;
  organization: string | null;
  timeoutSecs: number;
  targetLanguage: string;
  shortcut: string;
  apiKey: string | null;
  hasApiKey: boolean;
}

export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

export async function getDatabaseStatus(): Promise<DatabaseStatus> {
  return invoke<DatabaseStatus>("database_status");
}

export async function getProviderConfig(): Promise<ProviderConfigDto | null> {
  return invoke<ProviderConfigDto | null>("get_provider_config");
}

export async function saveProviderConfig(config: ProviderConfigDto): Promise<void> {
  await invoke("save_provider_config", { config });
}

export async function selectionPermissionStatus(): Promise<boolean> {
  return invoke<boolean>("selection_permission_status");
}

export async function openPopup(): Promise<void> {
  await invoke("open_popup");
}

export async function hidePopup(): Promise<void> {
  await invoke("hide_popup");
}

export async function latestPopupState(): Promise<PopupState | null> {
  return invoke<PopupState | null>("latest_popup_state");
}

/** 订阅 popup 状态事件；返回取消订阅函数。 */
export function onPopupState(handler: (state: PopupState) => void): Promise<() => void> {
  return listen<PopupState>("popup://state", (event) => handler(event.payload));
}
