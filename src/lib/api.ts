/**
 * 后端命令的类型化封装 —— 前端触达 Rust 的唯一通道。
 *
 * 约定：组件与 store 里不允许直接 `invoke()` / `listen()`，
 * 必须经过这里；命令名、参数与返回结构的契约集中在一个文件里。
 */
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";

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
  | { status: "loading"; sourceText: string | null }
  | ({ status: "result" } & TranslationResult)
  | { status: "error"; kind: string; message: string };

/** 与 Rust `TranslationResult` 对应（camelCase）。 */
export interface TranslationResult {
  sourceText: string;
  translatedText: string;
  detectedLanguage: string | null;
  alternatives: string[];
  explanation: string | null;
  dictionary: DictionaryEntryDto | null;
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
  /** 厂商扩展参数（JSON 对象文本），逐键合并进请求体。 */
  extraBody: string | null;
  apiKey: string | null;
  hasApiKey: boolean;
}

/** 应用设置：AI Router 路由 + 目标语言 + 全局快捷键。 */
export interface AppSettingsDto {
  defaultProviderId: string | null;
  translateProviderId: string | null;
  targetLanguage: string;
  shortcut: string;
  /** 界面主题：light | dark（主窗口与 Popup 共用）。 */
  uiTheme: string;
  ocrProvider: string;
  ocrTesseractPath: string;
  ocrHttpUrl: string;
}

export interface TestProviderResult {
  ok: boolean;
  latencyMs: number | null;
  reply: string | null;
  error: string | null;
}

/** 与 Rust `DictionaryDefinition` 对应。 */
export interface DictionaryDefinitionDto {
  partOfSpeech: string | null;
  definition: string;
  examples: string[];
  synonyms: string[];
  antonyms: string[];
}

/** 与 Rust `DictionaryEntry` 对应。 */
export interface DictionaryEntryDto {
  lemma: string;
  language: string;
  glossLanguage: string;
  pronunciations: string[];
  inflections: string[];
  source: string;
  definitions: DictionaryDefinitionDto[];
}

export interface DictionaryInfoDto {
  id: string;
  name: string;
  sourceLanguage: string;
  glossLanguage: string;
  entryCount: number;
}

export interface ImportDictionaryRequestDto {
  id: string;
  name: string;
  sourceLanguage: string;
  glossLanguage: string;
  path: string;
}

export interface ImportDictionaryResultDto {
  importedEntries: number;
  skippedLines: number;
}

export type Mastery = "new" | "learning" | "mastered";

export interface VocabularyItemDto {
  id: number;
  word: string;
  language: string;
  translation: string | null;
  notes: string;
  tags: string[];
  mastery: Mastery;
  lookupCount: number;
  reviewCount: number;
  source: string;
  examples: string[];
  createdAt: string;
  lastSeenAt: string;
}

export interface VocabularyFilterDto {
  search: string | null;
  tag: string | null;
  language: string | null;
  mastery: Mastery | null;
  limit: number;
}

export interface SaveVocabularyRequestDto {
  word: string;
  language: string | null;
  translation: string | null;
  source: string | null;
  examples: string[];
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

export async function screenRecordingStatus(): Promise<boolean> {
  return invoke<boolean>("screen_recording_status");
}

export async function startOcrCapture(): Promise<void> {
  await invoke("start_ocr_capture");
}

export async function getOcrBackground(): Promise<string | null> {
  return invoke<string | null>("get_ocr_background");
}

/** OCR 会话更新通知（每次开始截图都会触发）。返回取消订阅函数。 */
export function onOcrBackgroundUpdated(handler: () => void): Promise<() => void> {
  return listen("ocr://background-updated", () => handler());
}

/** 主题变更广播：主窗口保存设置后发，所有窗口应用新主题。 */
export async function broadcastThemeChanged(theme: string): Promise<void> {
  await emit("theme://changed", theme);
}

export function onThemeChanged(handler: (theme: string) => void): Promise<() => void> {
  return listen<string>("theme://changed", (event) => handler(event.payload));
}

/** 流式译文增量（payload 为到目前为止的完整译文，直接替换渲染）。 */
export function onPopupDelta(handler: (text: string) => void): Promise<() => void> {
  return listen<string>("popup://delta", (event) => handler(event.payload));
}

/** 流式译文最新累积文本（弹窗挂载晚于流开始时的追平用）。 */
export async function latestStreamed(): Promise<string> {
  return invoke<string>("latest_streamed");
}

export async function performOcrTranslation(
  x: number,
  y: number,
  width: number,
  height: number,
): Promise<void> {
  await invoke("perform_ocr_translation", { x, y, width, height });
}

export async function cancelOcr(): Promise<void> {
  await invoke("cancel_ocr");
}

export async function listDictionaries(): Promise<DictionaryInfoDto[]> {
  return invoke<DictionaryInfoDto[]>("list_dictionaries");
}

export async function importDictionaryTsv(
  request: ImportDictionaryRequestDto,
): Promise<ImportDictionaryResultDto> {
  return invoke<ImportDictionaryResultDto>("import_dictionary_tsv", { request });
}

export async function deleteDictionary(id: string): Promise<void> {
  await invoke("delete_dictionary", { id });
}

export async function listVocabulary(
  filter: VocabularyFilterDto,
): Promise<VocabularyItemDto[]> {
  return invoke<VocabularyItemDto[]>("list_vocabulary", { filter });
}

export async function updateVocabulary(
  id: number,
  notes: string,
  tags: string[],
  mastery: Mastery,
): Promise<void> {
  await invoke("update_vocabulary", { id, notes, tags, mastery });
}

export async function deleteVocabulary(id: number): Promise<void> {
  await invoke("delete_vocabulary", { id });
}

export async function getVocabularyTags(): Promise<string[]> {
  return invoke<string[]>("vocabulary_tags");
}

export async function saveToVocabulary(request: SaveVocabularyRequestDto): Promise<void> {
  await invoke("save_to_vocabulary", { request });
}

export async function openPopup(): Promise<void> {
  await invoke("open_popup");
}

export async function hidePopup(): Promise<void> {
  await invoke("hide_popup");
}

/** 切换 Popup 置顶（工具栏图钉）；返回切换后的状态。 */
export async function togglePopupPin(): Promise<boolean> {
  return invoke<boolean>("toggle_popup_pin");
}

export async function latestPopupState(): Promise<PopupState | null> {
  return invoke<PopupState | null>("latest_popup_state");
}

/** 订阅 popup 状态事件；返回取消订阅函数。 */
export function onPopupState(handler: (state: PopupState) => void): Promise<() => void> {
  return listen<PopupState>("popup://state", (event) => handler(event.payload));
}
