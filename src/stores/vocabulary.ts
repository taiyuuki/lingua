import { reactive, ref } from "vue";
import { defineStore } from "pinia";
import {
  deleteVocabulary,
  dueVocabularyCount,
  getVocabularyTags,
  gradeVocabularyReview,
  listDueVocabulary,
  listVocabulary,
  updateVocabulary,
  vocabularyBatchDelete,
  vocabularyBatchUpdate,
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

/** 复习一次取的队列长度：做多少算多少，不清空不惩罚。 */
export const REVIEW_QUEUE_SIZE = 20;

/** 掌握度循环顺序（点击徽章一键切换）。 */
const MASTERY_CYCLE: Record<Mastery, Mastery> = {
  new: "learning",
  learning: "mastered",
  mastered: "new",
};

/** 每页词条数：一页塞全部会让长释义列表完全失控。 */
export const VOCABULARY_PAGE_SIZE = 20;

/** 生词本域状态：筛选、分页、列表、编辑、批量操作。 */
export const useVocabularyStore = defineStore("vocabulary", () => {
  const items = ref<VocabularyItemDto[]>([]);
  const total = ref(0);
  const page = ref(1);
  const tags = ref<string[]>([]);
  const loaded = ref(false);
  const loadError = ref<string | null>(null);
  const editError = ref<string | null>(null);
  /** 掌握度统计（点击即筛）。 */
  const masteryCounts = reactive<Record<Mastery, number>>({
    new: 0,
    learning: 0,
    mastered: 0,
  });

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

  /** 拉当前页 + 总数 + 掌握度统计；页码越界（如删除后）自动回退。 */
  async function load(): Promise<void> {
    loadError.value = null;
    try {
      const countDto = { ...filterDto(), limit: 0, offset: 0 };
      total.value = await vocabularyCount(countDto);
      for (const option of MASTERY_OPTIONS) {
        masteryCounts[option.value] = await vocabularyCount({
          ...countDto,
          mastery: option.value,
        });
      }
      const maxPage = Math.max(1, Math.ceil(total.value / VOCABULARY_PAGE_SIZE));
      if (page.value > maxPage) {
        page.value = maxPage;
      }
      items.value = await listVocabulary(filterDto());
      tags.value = await getVocabularyTags();
      loaded.value = true;
      await refreshDueCount();
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

  /** 一键循环掌握度（点徽章）：保留笔记与标签。 */
  async function cycleMastery(item: VocabularyItemDto): Promise<void> {
    await saveEdit(item.id, item.notes, item.tags.join(", "), MASTERY_CYCLE[item.mastery]);
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

  /** 批量删除；清空选择并回到合法页码。 */
  async function batchRemove(ids: number[]): Promise<void> {
    try {
      await vocabularyBatchDelete(ids);
      await load();
    } catch (error) {
      loadError.value = describeError(error);
    }
  }

  /** 批量设掌握度 / 追加标签。 */
  async function batchUpdate(
    ids: number[],
    mastery: Mastery | null,
    addTags: string[] | null,
  ): Promise<void> {
    try {
      await vocabularyBatchUpdate(ids, mastery, addTags);
      await load();
    } catch (error) {
      loadError.value = describeError(error);
    }
  }

  // ---- 复习（SRS-lite）----

  const dueCount = ref(0);
  /** 复习模式：null = 列表模式。 */
  const reviewQueue = ref<VocabularyItemDto[]>([]);
  const reviewIndex = ref(0);
  const revealed = ref(false);
  const reviewStats = reactive({ again: 0, good: 0, easy: 0 });

  async function refreshDueCount(): Promise<void> {
    try {
      dueCount.value = await dueVocabularyCount();
    } catch {
      dueCount.value = 0;
    }
  }

  /** 进入复习模式：取到期队列（最多 20）。 */
  async function startReview(): Promise<void> {
    try {
      reviewQueue.value = await listDueVocabulary(REVIEW_QUEUE_SIZE);
      reviewIndex.value = 0;
      revealed.value = false;
      reviewStats.again = 0;
      reviewStats.good = 0;
      reviewStats.easy = 0;
    } catch (error) {
      loadError.value = describeError(error);
    }
  }

  function revealDefinition(): void {
    revealed.value = true;
  }

  /** 评分并前进；队列走完后停在总结页（endReview 返回列表）。 */
  async function grade(result: 0 | 1 | 2): Promise<void> {
    const item = reviewQueue.value[reviewIndex.value];
    if (!item) {
      return;
    }
    if (result === 0) {
      reviewStats.again += 1;
    } else if (result === 1) {
      reviewStats.good += 1;
    } else {
      reviewStats.easy += 1;
    }
    try {
      await gradeVocabularyReview(item.id, result);
    } catch (error) {
      loadError.value = describeError(error);
      return;
    }
    reviewIndex.value += 1;
    revealed.value = false;
  }

  async function endReview(): Promise<void> {
    reviewQueue.value = [];
    reviewIndex.value = 0;
    revealed.value = false;
    await Promise.all([load(), refreshDueCount()]);
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
    masteryCounts,
    MASTERY_OPTIONS,
    VOCABULARY_PAGE_SIZE,
    dueCount,
    reviewQueue,
    reviewIndex,
    revealed,
    reviewStats,
    load,
    applyFilters,
    turnTo,
    saveEdit,
    cycleMastery,
    remove,
    batchRemove,
    batchUpdate,
    refreshDueCount,
    startReview,
    revealDefinition,
    grade,
    endReview,
  };
});
