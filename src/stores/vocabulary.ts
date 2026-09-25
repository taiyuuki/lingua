import { reactive, ref } from "vue";
import { defineStore } from "pinia";
import {
  deleteVocabulary,
  getVocabularyTags,
  listVocabulary,
  updateVocabulary,
  vocabularyCount,
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

/** 每页词条数：一页塞全部会让长释义列表完全失控。 */
export const VOCABULARY_PAGE_SIZE = 20;

/** 生词本域状态：筛选、分页、列表、编辑。 */
export const useVocabularyStore = defineStore("vocabulary", () => {
  const items = ref<VocabularyItemDto[]>([]);
  const total = ref(0);
  const page = ref(1);
  const tags = ref<string[]>([]);
  const loaded = ref(false);
  const loadError = ref<string | null>(null);
  const editError = ref<string | null>(null);

  const filter = reactive({
    search: "",
    tag: "",
    mastery: "" as Mastery | "",
  });

  function filterDto(): VocabularyFilterDto {
    return {
      search: filter.search.trim() || null,
      tag: filter.tag || null,
      language: null,
      mastery: filter.mastery === "" ? null : filter.mastery,
      limit: VOCABULARY_PAGE_SIZE,
      offset: (page.value - 1) * VOCABULARY_PAGE_SIZE,
    };
  }

  /** 拉当前页 + 总数；页码越界（如删除后）自动回退。 */
  async function load(): Promise<void> {
    loadError.value = null;
    try {
      const countDto = { ...filterDto(), limit: 0, offset: 0 };
      total.value = await vocabularyCount(countDto);
      const maxPage = Math.max(1, Math.ceil(total.value / VOCABULARY_PAGE_SIZE));
      if (page.value > maxPage) {
        page.value = maxPage;
      }
      items.value = await listVocabulary(filterDto());
      tags.value = await getVocabularyTags();
      loaded.value = true;
    } catch (error) {
      loadError.value = describeError(error);
    }
  }

  /** 筛选变化：回第一页再拉。 */
  async function applyFilters(): Promise<void> {
    page.value = 1;
    await load();
  }

  async function turnTo(next: number): Promise<void> {
    page.value = next;
    await load();
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
    total,
    page,
    tags,
    loaded,
    loadError,
    editError,
    filter,
    MASTERY_OPTIONS,
    VOCABULARY_PAGE_SIZE,
    load,
    applyFilters,
    turnTo,
    saveEdit,
    remove,
  };
});
