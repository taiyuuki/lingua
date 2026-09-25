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
  /** 命中的词典词条（每本词典一条，按导入顺序，首页首选）；AI 结果为空。 */
  dictionaries: DictionaryEntryDto[];
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
  /** 能力声明；Vision 任务（OCR 兜底）只派给 canVision 的 Provider。 */
  canChat: boolean;
  canTranslate: boolean;
  canVision: boolean;
  /** 厂商扩展参数（JSON 对象文本），逐键合并进请求体。 */
  extraBody: string | null;
  apiKey: string | null;
  hasApiKey: boolean;
}

/** 应用设置：AI Router 路由 + 目标语言 + 全局快捷键。 */
export interface AppSettingsDto {
  defaultProviderId: string | null;
  translateProviderId: string | null;
  visionProviderId: string | null;
  targetLanguage: string;
  shortcut: string;
  /** 截图翻译快捷键（默认 alt+s；多显示器下跟随光标所在屏）。 */
  ocrShortcut: string;
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
  /** 导入期净化过的富文本 HTML（MDX 词库）；TSV 与旧数据为 null。 */
  html: string | null;
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
  /** MDX 释义引用了 CSS/图片资源但同目录没有配对 .mdd。 */
  resourcesMissing: boolean;
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
  offset: number;
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

/** 打开文件选择框选一个词典文件（TSV / MDX）；取消返回 null。 */
export async function pickDictionaryFilePath(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selection = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "词典文件", extensions: ["tsv", "txt", "mdx"] }],
  });
  return typeof selection === "string" ? selection : null;
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

/** 是否是可收藏的单词（短语/整段不支持）。 */
export async function isWord(text: string): Promise<boolean> {
  return invoke<boolean>("is_word", { text });
}

/** 生词本中是否已有该词。 */
export async function vocabularyWordStatus(
  word: string,
  language: string | null,
): Promise<boolean> {
  return invoke<boolean>("vocabulary_word_status", { word, language });
}

/** 从生词本移除该词。 */
export async function removeVocabularyWord(
  word: string,
  language: string | null,
): Promise<void> {
  await invoke("remove_vocabulary_word", { word, language });
}

export async function performOcrTranslation(
  x: number,
  y: number,
  width: number,
  height: number,
  viewportWidth: number,
  viewportHeight: number,
): Promise<void> {
  await invoke("perform_ocr_translation", {
    x,
    y,
    width,
    height,
    viewportWidth,
    viewportHeight,
  });
}

export async function cancelOcr(): Promise<void> {
  await invoke("cancel_ocr");
}

/** 上一次结果是否来自截图翻译且可用 Vision 手动重新识别。 */
export async function ocrRetryAvailable(): Promise<boolean> {
  return invoke<boolean>("ocr_retry_available");
}

/** 对保留的选区截图用 Vision 重新识别并重走翻译。 */
export async function retryOcrWithVision(): Promise<void> {
  await invoke("retry_ocr_with_vision");
}

/** 识别来源：哪个引擎产出文本、是否经过 Vision、置信度、兜底未生效原因。 */
export interface OcrSourceInfo {
  /** 如 tesseract-cli / http / vision:provider-id。 */
  engine: string;
  via: "primary" | "auto" | "manual";
  confidence: number | null;
  visionNote: string | null;
}

/** 最近一次截图翻译的识别来源；划词翻译结果为 null。 */
export async function ocrSourceInfo(): Promise<OcrSourceInfo | null> {
  return invoke<OcrSourceInfo | null>("ocr_source_info");
}

/** 开机自启状态（托盘菜单与设置页开关共用）。 */
export async function autostartEnabled(): Promise<boolean> {
  return invoke<boolean>("autostart_enabled");
}

/** 设置开机自启（同步托盘菜单勾选）。 */
export async function setAutostart(enabled: boolean): Promise<void> {
  await invoke("set_autostart", { enabled });
}

export async function listDictionaries(): Promise<DictionaryInfoDto[]> {
  return invoke<DictionaryInfoDto[]>("list_dictionaries");
}

export async function importDictionaryTsv(
  request: ImportDictionaryRequestDto,
): Promise<ImportDictionaryResultDto> {
  return invoke<ImportDictionaryResultDto>("import_dictionary_tsv", { request });
}

export async function importDictionaryMdx(
  request: ImportDictionaryRequestDto,
): Promise<ImportDictionaryResultDto> {
  return invoke<ImportDictionaryResultDto>("import_dictionary_mdx", { request });
}

/** MDX 导入进度事件 payload（大词库分批写库时上报）。 */
export interface DictionaryImportProgress {
  dictionaryId: string;
  /** parsing = 解析索引；importing = 批量写库。 */
  phase: "parsing" | "importing";
  imported: number;
  total: number;
}

/** 订阅 MDX 导入进度；返回取消订阅函数。 */
export function onDictionaryImportProgress(
  handler: (progress: DictionaryImportProgress) => void,
): Promise<() => void> {
  return listen<DictionaryImportProgress>("dictionary://import-progress", (event) =>
    handler(event.payload),
  );
}

export async function deleteDictionary(id: string): Promise<void> {
  await invoke("delete_dictionary", { id });
}

/** 词条内链跳词：按词头查词典（entry:// 链接点击用）；未命中返回 null。 */
export async function dictionaryLookup(
  word: string,
  language?: string | null,
): Promise<DictionaryEntryDto | null> {
  return invoke<DictionaryEntryDto | null>("dictionary_lookup", {
    word,
    language: language ?? null,
  });
}

/** 多词典词卡：按词头查全部命中词典（导入顺序，完整释义）。 */
export async function lookupWordDictionaries(
  word: string,
  language?: string | null,
): Promise<DictionaryEntryDto[]> {
  return invoke<DictionaryEntryDto[]>("lookup_word_dictionaries", {
    word,
    language: language ?? null,
  });
}

export async function listVocabulary(
  filter: VocabularyFilterDto,
): Promise<VocabularyItemDto[]> {
  return invoke<VocabularyItemDto[]>("list_vocabulary", { filter });
}

/** 与筛选条件匹配的词条总数（分页用）。 */
export async function vocabularyCount(
  filter: VocabularyFilterDto,
): Promise<number> {
  return invoke<number>("vocabulary_count", { filter });
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

/** 批量删除生词，返回实际删除条数。 */
export async function vocabularyBatchDelete(ids: number[]): Promise<number> {
  return invoke<number>("vocabulary_batch_delete", { ids });
}

/** 批量更新生词（掌握度和/或追加标签），返回实际更新条数。 */
export async function vocabularyBatchUpdate(
  ids: number[],
  mastery: Mastery | null,
  addTags: string[] | null,
): Promise<number> {
  return invoke<number>("vocabulary_batch_update", {
    ids,
    mastery,
    addTags,
  });
}

/** 复习评分（0=忘记 1=记得 2=简单）：打点 + SRS-lite 重排期。 */
export async function gradeVocabularyReview(
  id: number,
  result: 0 | 1 | 2,
): Promise<boolean> {
  return invoke<boolean>("grade_vocabulary_review", { id, result });
}

/** 到期待复习条数。 */
export async function dueVocabularyCount(): Promise<number> {
  return invoke<number>("due_vocabulary_count");
}

/** 到期复习队列（最早到期优先）。 */
export async function listDueVocabulary(limit: number): Promise<VocabularyItemDto[]> {
  return invoke<VocabularyItemDto[]>("list_due_vocabulary", { limit });
}

/** 按语言分组计数（生词本分组筛选用）。 */
export async function vocabularyLanguageCounts(): Promise<Array<[string, number]>> {
  return invoke<Array<[string, number]>>("vocabulary_language_counts");
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
