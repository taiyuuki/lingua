import { computed, reactive, ref } from "vue";
import { defineStore } from "pinia";
import {
  deleteDictionary,
  importDictionaryMdx,
  importDictionaryTsv,
  listDictionaries,
  onDictionaryImportProgress,
  type DictionaryImportProgress,
  type DictionaryInfoDto,
} from "../lib/api";
import { describeError } from "../lib/errors";

/** 词典导入表单。 */
export interface ImportForm {
  open: boolean;
  id: string;
  name: string;
  sourceLanguage: string;
  glossLanguage: string;
  path: string;
}

export const TSV_FORMAT_HINT = `TSV 格式（TAB 分隔，# 开头为注释）：
lemma	发音	词性	释义	例句1 / 例句2	同义词1 / 同义词2	反义词
后三列可省略；同一 lemma 多行会合并为一个词条。`;

export const MDX_FORMAT_HINT = `MDX（MDict）词库：
· 无加密与固定密钥加密（Encrypted=2，含多数牛津词库）均可导入；
· 释义按富文本渲染：同目录同名 .mdd 的 CSS/图片会一并导入；
· 早期导入的词库无富文本，删除后重新导入即可；
· 绑定用户注册信息的加密（Encrypted 含 1）无法离线解密，不支持；
· 大词库分批写入并显示进度，导入中请勿关闭应用。`;

/** 是否按 MDX 词库导入（按扩展名分发）。 */
export function isMdxPath(path: string): boolean {
  return path.trim().toLowerCase().endsWith(".mdx");
}

function emptyForm(): ImportForm {
  return {
    open: false,
    id: "",
    name: "",
    sourceLanguage: "en",
    glossLanguage: "zh",
    path: "",
  };
}

/** 词典管理域状态：列表 + 导入表单。 */
export const useDictionariesStore = defineStore("dictionaries", () => {
  const dictionaries = ref<DictionaryInfoDto[]>([]);
  const loaded = ref(false);
  const loadError = ref<string | null>(null);
  const importError = ref<string | null>(null);
  const importMessage = ref<string | null>(null);
  const importing = ref(false);
  /** MDX 导入进度（TSV 导入恒为 null）。 */
  const importProgress = ref<DictionaryImportProgress | null>(null);
  const form = reactive<ImportForm>(emptyForm());

  const hasDictionaries = computed(() => dictionaries.value.length > 0);

  async function load(): Promise<void> {
    loadError.value = null;
    try {
      dictionaries.value = await listDictionaries();
      loaded.value = true;
    } catch (error) {
      loadError.value = describeError(error);
    }
  }

  function openImport(): void {
    Object.assign(form, emptyForm(), { open: true });
    importError.value = null;
    importMessage.value = null;
  }

  function closeForm(): void {
    form.open = false;
  }

  /** 导入词典：.mdx 走 MDX 命令（带进度事件），其余按 TSV。 */
  async function importDictionary(): Promise<void> {
    importing.value = true;
    importError.value = null;
    importMessage.value = null;
    importProgress.value = null;
    const unsubscribe = await onDictionaryImportProgress((progress) => {
      importProgress.value = progress;
    });
    try {
      const request = {
        id: form.id.trim(),
        name: form.name.trim() || form.id.trim(),
        sourceLanguage: form.sourceLanguage,
        glossLanguage: form.glossLanguage,
        path: form.path.trim(),
      };
      const result = isMdxPath(form.path)
        ? await importDictionaryMdx(request)
        : await importDictionaryTsv(request);
      importMessage.value = `导入成功：${result.importedEntries} 个词条` +
        (result.skippedLines > 0 ? `，跳过 ${result.skippedLines} 条` : "");
      await load();
    } catch (error) {
      importError.value = describeError(error);
    } finally {
      unsubscribe();
      importProgress.value = null;
      importing.value = false;
    }
  }

  async function remove(id: string): Promise<void> {
    // 确认交互在组件层（两段点击）；WKWebView 不支持 window.confirm。
    try {
      await deleteDictionary(id);
      await load();
    } catch (error) {
      loadError.value = describeError(error);
    }
  }

  return {
    dictionaries,
    loaded,
    loadError,
    importError,
    importMessage,
    importing,
    importProgress,
    form,
    hasDictionaries,
    load,
    openImport,
    closeForm,
    importDictionary,
    remove,
  };
});
