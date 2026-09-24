import { computed, reactive, ref } from "vue";
import { defineStore } from "pinia";
import {
  deleteVocabulary,
  getVocabularyTags,
  listVocabulary,
  updateVocabulary,
  type Mastery,
  type VocabularyFilterDto,
  type VocabularyItemDto,
} from "../lib/api";
import { describeError } from "../lib/errors";

export const MASTERY_OPTIONS: Array<{ value: Mastery; label: string }> = [
  { value: "new", label: "新词" },
  { value: "learning", label: "学习中" },
  { value: "mastered", label: "已掌握" },
];

/** 生词本域状态：筛选、列表、编辑。 */
export const useVocabularyStore = defineStore("vocabulary", () => {
  const items = ref<VocabularyItemDto[]>([]);
  const tags = ref<string[]>([]);
  const loaded = ref(false);
  const loadError = ref<string | null>(null);
  const editError = ref<string | null>(null);

  const filter = reactive({ search: "", tag: "", mastery: "" as Mastery | "" });

  const totalCount = computed(() => items.value.length);

  async function load(): Promise<void> {
    loadError.value = null;
    try {
      const filterDto: VocabularyFilterDto = {
        search: filter.search.trim() || null,
        tag: filter.tag || null,
        language: null,
        mastery: (filter.mastery || null) as VocabularyFilterDto["mastery"],
        limit: 0,
      };
      items.value = await listVocabulary(filterDto);
      tags.value = await getVocabularyTags();
      loaded.value = true;
    } catch (error) {
      loadError.value = describeError(error);
    }
  }

  async function saveEdit(
    id: number,
    notes: string,
    tagsInput: string,
    mastery: Mastery,
  ): Promise<void> {
    editError.value = null;
    try {
      const tags = tagsInput
        .split(/[,,]/)
        .map((tag) => tag.trim())
        .filter((tag) => tag !== "");
      await updateVocabulary(id, notes, tags, mastery);
      await load();
    } catch (error) {
      editError.value = describeError(error);
    }
  }

  async function remove(id: number): Promise<void> {
    // 确认交互在组件层（两段点击）；WKWebView 不支持 window.confirm。
    try {
      await deleteVocabulary(id);
      await load();
    } catch (error) {
      loadError.value = describeError(error);
    }
  }

  return {
    items,
    tags,
    loaded,
    loadError,
    editError,
    filter,
    totalCount,
    MASTERY_OPTIONS,
    load,
    saveEdit,
    remove,
  };
});
