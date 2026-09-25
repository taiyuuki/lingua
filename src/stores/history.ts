import { reactive, ref } from "vue";
import { defineStore } from "pinia";
import {
  clearHistory,
  deleteHistory,
  historyCount,
  listHistory,
  type HistoryFilterDto,
  type HistoryItemDto,
  type HistoryKind,
} from "../lib/api";
import { describeError } from "../lib/errors";

/** 每页条数。 */
export const HISTORY_PAGE_SIZE = 30;

/** 来源筛选项（chips 用）。 */
export const HISTORY_KIND_OPTIONS: Array<{ value: HistoryKind; label: string }> = [
  { value: "selection", label: "划词" },
  { value: "ocr", label: "截图" },
];

/** 翻译历史域状态：筛选、分页、列表、删除、清空。 */
export const useHistoryStore = defineStore("history", () => {
  const items = ref<HistoryItemDto[]>([]);
  const total = ref(0);
  const page = ref(1);
  const loaded = ref(false);
  const loadError = ref<string | null>(null);

  const filter = reactive({
    search: "",
    kind: "" as HistoryKind | "",
  });

  function filterDto(): HistoryFilterDto {
    return {
      search: filter.search.trim() || null,
      kind: filter.kind === "" ? null : filter.kind,
      limit: HISTORY_PAGE_SIZE,
      offset: (page.value - 1) * HISTORY_PAGE_SIZE,
    };
  }

  /** 拉当前页 + 总数；页码越界（如删除后）自动回退。 */
  async function load(): Promise<void> {
    loadError.value = null;
    try {
      total.value = await historyCount({ ...filterDto(), limit: 0, offset: 0 });
      const maxPage = Math.max(1, Math.ceil(total.value / HISTORY_PAGE_SIZE));
      if (page.value > maxPage) {
        page.value = maxPage;
      }
      items.value = await listHistory(filterDto());
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

  async function remove(id: number): Promise<void> {
    try {
      await deleteHistory(id);
      await load();
    } catch (error) {
      loadError.value = describeError(error);
    }
  }

  /** 清空全部历史（组件层两段点击确认）。返回删除条数。 */
  async function clear(): Promise<number> {
    try {
      const removed = await clearHistory();
      await load();
      return removed;
    } catch (error) {
      loadError.value = describeError(error);
      return 0;
    }
  }

  return {
    items,
    total,
    page,
    loaded,
    loadError,
    filter,
    HISTORY_PAGE_SIZE,
    load,
    applyFilters,
    turnTo,
    remove,
    clear,
  };
});
