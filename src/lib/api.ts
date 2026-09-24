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
export interface ProviderDto {
  id: string;
  name: string;
  kind: "openai_compatible" | "openai";
  baseUrl: string;
  model: string | null;
  organization: string | null;
  timeoutSecs: number;
  apiKey: string | null;
  hasApiKey: boolean;
}

/** 应用设置：AI Router 路由 + 目标语言 + 全局快捷键。 */
export interface AppSettingsDto {
  defaultProviderId: string | null;
  translateProviderId: string | null;
  targetLanguage: string;
  shortcut: string;
}

export interface TestProviderResult {
  ok: boolean;
  latencyMs: number | null;
  reply: string | null;
  error: string | null;
}

export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

export async function getDatabaseStatus(): Promise<DatabaseStatus> {
  return invoke<DatabaseStatus>("database_status");
}

export async function listProviders(): Promise<ProviderDto[]> {
  return invoke<ProviderDto[]>("list_providers");
}

export async function saveProvider(provider: ProviderDto): Promise<void> {
  await invoke("save_provider", { provider });
}

export async function deleteProvider(id: string): Promise<void> {
  await invoke("delete_provider", { id });
}

export async function testProvider(id: string): Promise<TestProviderResult> {
  return invoke<TestProviderResult>("test_provider", { id });
}

export async function getAppSettings(): Promise<AppSettingsDto> {
  return invoke<AppSettingsDto>("get_app_settings");
}

export async function saveAppSettings(settings: AppSettingsDto): Promise<void> {
  await invoke("save_app_settings", { settings });
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
