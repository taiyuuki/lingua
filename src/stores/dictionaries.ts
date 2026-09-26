import { computed, reactive, ref } from "vue";
import { defineStore } from "pinia";
import {
  cancelDictionaryImport,
  deleteAudioLibrary,
  deleteDictionary,
  importAudioLibrary,
  importDictionaryMdx,
  importDictionaryTsv,
  listAudioLibraries,
  listDictionaries,
  reorderDictionaries,
  onDictionaryImportProgress,
  type AudioLibraryDto,
  type DictionaryImportProgress,
  type DictionaryInfoDto,
} from "../lib/api";
import { describeError } from "../lib/errors";

/** 词典导入表单（独立导入页使用）。id 由后端按文件名自动生成，不再要求手填。 */
export interface ImportForm {
  name: string;
  sourceLanguage: string;
  glossLanguage: string;
  path: string;
}

export const TSV_FORMAT_HINT = `TSV 格式（TAB 分隔，# 开头为注释）：
lemma	发音	词性	释义	例句1 / 例句2	同义词1 / 同义词2	反义词
后三列可省略；同一 lemma 多行会合并为一个词条。`;

export const MDX_FORMAT_HINT = `MDX（MDict）词库：
· 常见的 MDX 词典均可导入（含多数牛津词库）；
· 把同名 .mdd 文件放在 .mdx 旁边，释义可完整显示图片与排版；
· 少数加密词典暂不支持导入；
· 大词库导入需要一些时间，导入过程中请勿关闭应用。`;

/** 是否按 MDX 词库导入（按扩展名分发）。 */
export function isMdxPath(path: string): boolean {
  return path.trim().toLowerCase().endsWith(".mdx");
}

function emptyForm(): ImportForm {
  return {
    name: "",
    sourceLanguage: "en",
    glossLanguage: "zh",
    path: "",
  };
}

/** 从路径取文件名（去扩展名）；取不到返回空串。 */
export function fileNameStem(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? "";
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}

/** 词典管理域状态：列表 + 导入表单。 */
export const useDictionariesStore = defineStore("dictionaries", () => {
  const dictionaries = ref<DictionaryInfoDto[]>([]);
  const loaded = ref(false);
  /** 列表首载中（首屏动画用）。 */
  const loading = ref(false);
  const loadError = ref<string | null>(null);
  const importError = ref<string | null>(null);
  const importMessage = ref<string | null>(null);
  /** MDX 引用了资源但没有配对 .mdd 的导入警告（与错误分开显示）。 */
  const importWarning = ref<string | null>(null);
  const importing = ref(false);
  /** 取消请求已发出（等待后端批边界停止并清理）。 */
  const cancelling = ref(false);
  /** 正在删除的词典 id（行级 loading）。 */
  const deletingId = ref<string | null>(null);
  /** MDX 导入进度（TSV 导入恒为 null）。 */
  const importProgress = ref<DictionaryImportProgress | null>(null);
  const form = reactive<ImportForm>(emptyForm());

  const hasDictionaries = computed(() => dictionaries.value.length > 0);

  async function load(): Promise<void> {
    if (!loaded.value) {
      loading.value = true;
    }
    loadError.value = null;
    try {
      dictionaries.value = await listDictionaries();
      loaded.value = true;
    } catch (error) {
      loadError.value = describeError(error);
    } finally {
      loading.value = false;
    }
  }

  /** 进入导入页时调用：空闲则重置表单与提示；导入进行中保留现场（进度与结果提示不动）。 */
  function beginImport(): void {
    if (importing.value) {
      return;
    }
    Object.assign(form, emptyForm());
    importError.value = null;
    importMessage.value = null;
    importWarning.value = null;
  }

  /** 上次自动填入的名称：换选文件时仅当名称仍是自动值才覆盖，
   *  用户手改过的名称不被文件选择冲掉。 */
  let autoFilledName = "";

  /** 选定词典文件后填路径；名称为空或仍是自动值时用文件名填入。 */
  function setPickedPath(path: string): void {
    form.path = path;
    if (!form.name || form.name === autoFilledName) {
      autoFilledName = fileNameStem(path);
      form.name = autoFilledName;
    }
  }

  /** 导入词典：.mdx 走 MDX 命令（带进度事件），其余按 TSV。 */
  async function importDictionary(): Promise<void> {
    importing.value = true;
    importError.value = null;
    importMessage.value = null;
    importWarning.value = null;
    importProgress.value = null;
    const unsubscribe = await onDictionaryImportProgress((progress) => {
      importProgress.value = progress;
    });
    try {
      const request = {
        // id 留空：后端按文件名自动生成（同文件重导 → 覆盖更新）。
        id: "",
        name: form.name.trim(),
        sourceLanguage: form.sourceLanguage,
        glossLanguage: form.glossLanguage,
        path: form.path.trim(),
      };
      const result = isMdxPath(form.path)
        ? await importDictionaryMdx(request)
        : await importDictionaryTsv(request);
      if (result.cancelled) {
        importMessage.value = "已取消导入，本次的词条与资源已清理。";
      } else {
        importMessage.value = `导入成功：${result.importedEntries} 个词条` +
          (result.skippedLines > 0 ? `，跳过 ${result.skippedLines} 条` : "");
        if (result.resourcesMissing) {
          importWarning.value =
            "该词典缺少配对的 .mdd 资源文件，释义中的图片无法显示。" +
            "把 .mdd 放到 .mdx 同一目录后，删除并重新导入即可。";
        }
      }
      await load();
    } catch (error) {
      importError.value = describeError(error);
    } finally {
      unsubscribe();
      importProgress.value = null;
      importing.value = false;
      cancelling.value = false;
    }
  }

  /** 取消导入：后端在批边界停止并清理半成品，本次 import 调用随即返回。 */
  async function cancelImport(): Promise<void> {
    cancelling.value = true;
    try {
      await cancelDictionaryImport();
    } catch {
      cancelling.value = false;
    }
  }

  async function remove(id: string): Promise<void> {
    // 确认交互在组件层（两段点击）；WKWebView 不支持 window.confirm。
    deletingId.value = id;
    try {
      await deleteDictionary(id);
      await load();
    } catch (error) {
      loadError.value = describeError(error);
    } finally {
      deletingId.value = null;
    }
  }

  /** 应用拖拽后的完整顺序：本地立即生效，再整表重写排序；失败提示并回滚重拉。 */
  async function applyOrder(next: DictionaryInfoDto[]): Promise<void> {
    const prev = dictionaries.value;
    const unchanged = next.length === prev.length &&
      next.every((dict, i) => dict.id === prev[i]?.id);
    if (unchanged) {
      return;
    }
    dictionaries.value = next;
    try {
      await reorderDictionaries(next.map((dict) => dict.id));
    } catch (error) {
      loadError.value = describeError(error);
      await load();
    }
  }

  // ---------- 发音库 ----------

  const audioLibraries = ref<AudioLibraryDto[]>([]);
  const audioLoading = ref(false);
  /** 发音库导入中（复制 + 建索引，秒级，无进度事件）。 */
  const audioImporting = ref(false);
  const audioError = ref<string | null>(null);
  const audioMessage = ref<string | null>(null);
  /** 正在删除的发音库 id（行级 loading）。 */
  const audioDeletingId = ref<string | null>(null);

  async function loadAudioLibraries(): Promise<void> {
    audioLoading.value = true;
    audioError.value = null;
    try {
      audioLibraries.value = await listAudioLibraries();
    } catch (error) {
      audioError.value = describeError(error);
    } finally {
      audioLoading.value = false;
    }
  }

  /** 导入发音库；成功返回 true（提示信息已写入 audioMessage）。 */
  async function importAudio(path: string): Promise<boolean> {
    audioImporting.value = true;
    audioError.value = null;
    audioMessage.value = null;
    try {
      const library = await importAudioLibrary(path);
      audioMessage.value = `导入成功：${library.name}，${library.entryCount} 个词头音频` +
        `（${library.accents.map((accent) => (accent === "uk" ? "英" : "美")).join(" / ")}音）`;
      await loadAudioLibraries();
      return true;
    } catch (error) {
      audioError.value = describeError(error);
      return false;
    } finally {
      audioImporting.value = false;
    }
  }

  async function removeAudio(id: string): Promise<void> {
    audioDeletingId.value = id;
    try {
      await deleteAudioLibrary(id);
      await loadAudioLibraries();
    } catch (error) {
      audioError.value = describeError(error);
    } finally {
      audioDeletingId.value = null;
    }
  }

  return {
    dictionaries,
    loaded,
    loading,
    loadError,
    importError,
    importMessage,
    importWarning,
    importing,
    cancelling,
    deletingId,
    importProgress,
    form,
    hasDictionaries,
    load,
    beginImport,
    setPickedPath,
    importDictionary,
    cancelImport,
    remove,
    applyOrder,
    audioLibraries,
    audioLoading,
    audioImporting,
    audioError,
    audioMessage,
    audioDeletingId,
    loadAudioLibraries,
    importAudio,
    removeAudio,
  };
});
