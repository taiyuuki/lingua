import { computed, reactive, ref } from "vue";
import { defineStore } from "pinia";
import {
  deleteDictionary,
  importDictionaryTsv,
  listDictionaries,
  type DictionaryInfoDto,
} from "../lib/api";
import { describeError } from "../lib/errors";
import { SUPPORTED_TARGET_LANGUAGES } from "./settings";

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
  const form = reactive<ImportForm>(emptyForm());

  const languageOptions = SUPPORTED_TARGET_LANGUAGES;
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

  async function importTsv(): Promise<void> {
    importing.value = true;
    importError.value = null;
    importMessage.value = null;
    try {
      const result = await importDictionaryTsv({
        id: form.id.trim(),
        name: form.name.trim() || form.id.trim(),
        sourceLanguage: form.sourceLanguage,
        glossLanguage: form.glossLanguage,
        path: form.path.trim(),
      });
      importMessage.value = `导入成功：${result.importedEntries} 个词条` +
        (result.skippedLines > 0 ? `，跳过 ${result.skippedLines} 行` : "");
      await load();
    } catch (error) {
      importError.value = describeError(error);
    } finally {
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
    form,
    languageOptions,
    hasDictionaries,
    load,
    openImport,
    closeForm,
    importTsv,
    remove,
  };
});
