<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { storeToRefs } from "pinia";
import {
  NAlert,
  NButton,
  NCard,
  NCheckbox,
  NEmpty,
  NForm,
  NFormItem,
  NInput,
  NList,
  NListItem,
  NPagination,
  NPopconfirm,
  NSelect,
  NSpace,
  NTag,
  NText,
} from "naive-ui";
import {
  MASTERY_OPTIONS,
  VOCABULARY_PAGE_SIZE,
  useVocabularyStore,
} from "../stores/vocabulary";
import { lookupEntriesForWord } from "../lib/dictRender";
import type {
  DictionaryEntryDto,
  Mastery,
  VocabularyItemDto,
} from "../lib/api";
import DictionaryEntryPanel from "./DictionaryEntryPanel.vue";

const store = useVocabularyStore();
const {
  items,
  total,
  page,
  tags,
  loadError,
  editError,
  filter,
  masteryCounts,
} = storeToRefs(store);

const editingId = ref<number | null>(null);
const editForm = reactive({ notes: "", tags: "", mastery: "new" as Mastery });

// ---- 词卡（点词头展开多词典释义）----
const expandedCardId = ref<number | null>(null);
const cardEntries = ref<Record<number, DictionaryEntryDto[]>>({});

async function toggleCard(item: VocabularyItemDto): Promise<void> {
  if (expandedCardId.value === item.id) {
    expandedCardId.value = null;
    return;
  }
  expandedCardId.value = item.id;
  if (!cardEntries.value[item.id]) {
    cardEntries.value = {
      ...cardEntries.value,
      [item.id]: await lookupEntriesForWord(item.word, item.language),
    };
  }
}

/** 词卡内 entry:// 跳词：替换该卡的词典数据。 */
async function cardLookupWord(word: string): Promise<void> {
  const id = expandedCardId.value;
  if (id === null || !word) {
    return;
  }
  const entries = await lookupEntriesForWord(word);
  if (entries.length > 0) {
    cardEntries.value = { ...cardEntries.value, [id]: entries };
  }
}

// ---- 掌握度 ----
const masteryLabel = (mastery: Mastery): string =>
  MASTERY_OPTIONS.find((option) => option.value === mastery)?.label ?? mastery;

const masteryOptions = MASTERY_OPTIONS.map((option) => ({
  label: option.label,
  value: option.value,
}));

const masteryStats = computed(() =>
  MASTERY_OPTIONS.map((option) => ({
    ...option,
    count: masteryCounts.value[option.value],
  })),
);

function toggleMasteryFilter(mastery: Mastery): void {
  filter.value.mastery = filter.value.mastery === mastery ? "" : mastery;
  void store.applyFilters();
}

// ---- 标签 chips ----
function toggleTagFilter(tag: string): void {
  filter.value.tag = filter.value.tag === tag ? "" : tag;
  void store.applyFilters();
}

// ---- 批量操作 ----
const selectedIds = ref(new Set<number>());
const batchTagInput = ref("");
const batchMastery = ref<Mastery | null>(null);

const hasSelection = computed(() => selectedIds.value.size > 0);

function toggleSelected(id: number, checked: boolean): void {
  const next = new Set(selectedIds.value);
  if (checked) {
    next.add(id);
  } else {
    next.delete(id);
  }
  selectedIds.value = next;
}

function clearSelection(): void {
  selectedIds.value = new Set();
  batchTagInput.value = "";
  batchMastery.value = null;
}

async function batchDelete(): Promise<void> {
  await store.batchRemove([...selectedIds.value]);
  clearSelection();
}

async function batchApplyMastery(): Promise<void> {
  if (!batchMastery.value) {
    return;
  }
  await store.batchUpdate([...selectedIds.value], batchMastery.value, null);
  clearSelection();
}

async function batchApplyTags(): Promise<void> {
  const addTags = batchTagInput.value
    .split(/[,,]/)
    .map((tag) => tag.trim())
    .filter((tag) => tag !== "");
  if (addTags.length === 0) {
    return;
  }
  await store.batchUpdate([...selectedIds.value], null, addTags);
  batchTagInput.value = "";
  clearSelection();
}

// ---- 编辑 ----
function startEdit(item: VocabularyItemDto): void {
  editingId.value = item.id;
  editForm.notes = item.notes;
  editForm.tags = item.tags.join(", ");
  editForm.mastery = item.mastery;
}

async function submitEdit(id: number): Promise<void> {
  await store.saveEdit(id, editForm.notes, editForm.tags, editForm.mastery);
  if (!editError.value) {
    editingId.value = null;
  }
}

const pageCount = computed(() =>
  Math.max(1, Math.ceil(total.value / VOCABULARY_PAGE_SIZE)),
);

onMounted(() => {
  void store.load();
});
</script>

<template>
  <n-card
    title="生词本"
    size="small"
  >
    <template #header-extra>
      <n-text depth="3">
        共 {{ total }} 条
      </n-text>
    </template>

    <n-text
      depth="3"
      style="display: block; margin-bottom: 12px"
    >
      划词翻译单词时自动记录；点词头展开词典释义，点掌握度徽章一键切换。
    </n-text>
    <n-alert
      v-if="loadError"
      type="error"
      :show-icon="true"
      style="margin-bottom: 12px"
    >
      {{ loadError }}
    </n-alert>

    <!-- 筛选：搜索 + 掌握度统计（点击即筛）+ 标签 chips -->
    <n-input
      v-model:value="filter.search"
      placeholder="搜索（词 / 译文 / 笔记），回车应用"
      clearable
      style="margin-bottom: 10px"
      @keydown.enter="store.applyFilters()"
      @blur="store.applyFilters()"
    />
    <div class="vocab-stat-row">
      <button
        type="button"
        :class="['vocab-stat', { active: filter.mastery === '' }]"
        @click="filter.mastery = ''; store.applyFilters()"
      >
        全部 {{ total }}
      </button>
      <button
        v-for="stat in masteryStats"
        :key="stat.value"
        type="button"
        :class="['vocab-stat', `is-${stat.value}`, { active: filter.mastery === stat.value }]"
        @click="toggleMasteryFilter(stat.value)"
      >
        {{ stat.label }} {{ stat.count }}
      </button>
    </div>
    <div
      v-if="tags.length > 0"
      class="vocab-tag-row"
    >
      <button
        type="button"
        :class="['vocab-tag-chip', { active: filter.tag === '' }]"
        @click="filter.tag = ''; store.applyFilters()"
      >
        全部标签
      </button>
      <button
        v-for="tag in tags"
        :key="tag"
        type="button"
        :class="['vocab-tag-chip', { active: filter.tag === tag }]"
        @click="toggleTagFilter(tag)"
      >
        {{ tag }}
      </button>
    </div>

    <!-- 批量操作条 -->
    <div
      v-if="hasSelection"
      class="vocab-batch-bar"
    >
      <n-text
        depth="3"
        style="flex-shrink: 0"
      >
        已选 {{ selectedIds.size }} 条
      </n-text>
      <n-select
        v-model:value="batchMastery"
        :options="masteryOptions"
        placeholder="设掌握度…"
        size="small"
        style="width: 120px"
        clearable
      />
      <n-button
        size="small"
        secondary
        :disabled="!batchMastery"
        @click="batchApplyMastery"
      >
        应用
      </n-button>
      <n-input
        v-model:value="batchTagInput"
        size="small"
        placeholder="追加标签（逗号分隔）"
        style="width: 170px"
        @keydown.enter="batchApplyTags"
      />
      <n-button
        size="small"
        secondary
        :disabled="!batchTagInput.trim()"
        @click="batchApplyTags"
      >
        加标签
      </n-button>
      <n-popconfirm
        :show-icon="false"
        @positive-click="batchDelete"
      >
        <template #trigger>
          <n-button
            size="small"
            type="error"
            secondary
          >
            删除
          </n-button>
        </template>
        删除选中的 {{ selectedIds.size }} 条生词，确定？
      </n-popconfirm>
      <n-button
        size="small"
        quaternary
        @click="clearSelection"
      >
        取消
      </n-button>
    </div>

    <!-- 词条列表 -->
    <n-list
      v-if="items.length > 0"
      :show-divider="true"
    >
      <n-list-item
        v-for="item in items"
        :key="item.id"
      >
        <!-- 编辑态 -->
        <n-form
          v-if="editingId === item.id"
          label-placement="top"
          size="small"
          :show-feedback="false"
        >
          <n-space
            vertical
            :size="12"
          >
            <n-text strong>
              {{ item.word }}
            </n-text>
            <n-form-item label="笔记">
              <n-input
                v-model:value="editForm.notes"
                type="textarea"
                :rows="2"
                placeholder="这个词对你的意义、联想、易错点…"
              />
            </n-form-item>
            <div class="form-grid">
              <n-form-item label="标签（逗号分隔）">
                <n-input
                  v-model:value="editForm.tags"
                  placeholder="gre, noun"
                />
              </n-form-item>
              <n-form-item label="掌握度">
                <n-select
                  v-model:value="editForm.mastery"
                  :options="masteryOptions"
                />
              </n-form-item>
            </div>
            <n-text
              v-if="item.examples.length > 0"
              depth="3"
              class="clamp-2"
              style="font-size: 12px"
              :title="item.examples.join('\n')"
            >
              例句：{{ item.examples.join(" / ") }}
            </n-text>
            <n-space>
              <n-button
                type="primary"
                @click="submitEdit(item.id)"
              >
                保存
              </n-button>
              <n-button
                secondary
                @click="editingId = null"
              >
                取消
              </n-button>
            </n-space>
            <n-alert
              v-if="editError"
              type="error"
              :show-icon="true"
            >
              {{ editError }}
            </n-alert>
          </n-space>
        </n-form>

        <!-- 浏览态 -->
        <div
          v-else
          class="vocab-item"
        >
          <div class="vocab-head">
            <n-checkbox
              :checked="selectedIds.has(item.id)"
              @update:checked="toggleSelected(item.id, $event)"
            />
            <button
              type="button"
              class="vocab-word"
              :title="`查看「${item.word}」的词典释义`"
              @click="toggleCard(item)"
            >
              {{ item.word }}
            </button>
            <button
              type="button"
              class="vocab-mastery"
              :class="`is-${item.mastery}`"
              title="点击切换掌握度"
              @click="store.cycleMastery(item)"
            >
              {{ masteryLabel(item.mastery) }}
            </button>
            <n-tag
              v-for="tag in item.tags"
              :key="tag"
              size="small"
              :bordered="false"
            >
              {{ tag }}
            </n-tag>
          </div>

          <div class="vocab-body">
            <p
              v-if="item.translation"
              class="vocab-translation clamp-2"
              :title="item.translation"
            >
              {{ item.translation }}
            </p>
            <p
              v-if="item.notes"
              class="vocab-notes clamp-2"
              :title="item.notes"
            >
              {{ item.notes }}
            </p>
          </div>

          <!-- 词卡：多词典完整释义（富文本/发音/内链），点词头开合 -->
          <div
            v-if="expandedCardId === item.id"
            class="vocab-card"
          >
            <n-text
              v-if="(cardEntries[item.id] ?? []).length === 0"
              depth="3"
              style="font-size: 12.5px"
            >
              没有词典收录「{{ item.word }}」。
            </n-text>
            <DictionaryEntryPanel
              v-else
              :entries="cardEntries[item.id] ?? []"
              @lookup-word="cardLookupWord"
            />
          </div>

          <div class="vocab-foot">
            <span class="vocab-meta clamp-1">
              查询 {{ item.lookupCount }} · 复习 {{ item.reviewCount }} ·
              {{ item.lastSeenAt.slice(0, 10) }} · {{ item.source }}
            </span>
            <span class="vocab-actions">
              <n-button
                size="tiny"
                secondary
                @click="startEdit(item)"
              >
                编辑
              </n-button>
              <n-popconfirm
                :show-icon="false"
                @positive-click="store.remove(item.id)"
              >
                <template #trigger>
                  <n-button
                    size="tiny"
                    type="error"
                    secondary
                  >
                    删除
                  </n-button>
                </template>
                从生词本移除该词条，确定？
              </n-popconfirm>
            </span>
          </div>
        </div>
      </n-list-item>
    </n-list>
    <n-empty
      v-else-if="!loadError"
      description="还没有生词记录。用 alt+t 翻译一个单词试试。"
      style="margin: 12px 0"
    />

    <!-- 分页 -->
    <div
      v-if="pageCount > 1"
      class="vocab-pagination"
    >
      <n-pagination
        :page="page"
        :page-count="pageCount"
        :page-size="VOCABULARY_PAGE_SIZE"
        :item-count="total"
        @update:page="store.turnTo($event)"
      />
    </div>
  </n-card>
</template>

<style scoped>
/* 掌握度统计条：点击即筛 */
.vocab-stat-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 8px;
}

.vocab-stat {
  padding: 2px 10px;
  border-radius: 999px;
  border: 1px solid var(--color-border);
  background: transparent;
  color: var(--color-text-secondary);
  font-size: 12px;
  cursor: pointer;
}

.vocab-stat:hover {
  color: var(--color-text);
}

.vocab-stat.active {
  border-color: var(--color-accent);
  color: var(--color-accent);
  background: color-mix(in srgb, var(--color-accent) 10%, transparent);
}

/* 标签 chips */
.vocab-tag-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 12px;
}

.vocab-tag-chip {
  padding: 1px 8px;
  border-radius: 6px;
  border: 1px dashed var(--color-border);
  background: transparent;
  color: var(--color-text-secondary);
  font-size: 11.5px;
  cursor: pointer;
}

.vocab-tag-chip.active {
  border-style: solid;
  border-color: var(--color-accent);
  color: var(--color-accent);
}

/* 批量操作条 */
.vocab-batch-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  margin-bottom: 12px;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  background: color-mix(in srgb, var(--color-accent) 5%, var(--color-surface));
}

/* 词条卡片：垂直三层 + 可选中复制 */
.vocab-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
  user-select: text;
}

.vocab-item :deep(button) {
  user-select: none;
}

.vocab-head {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  min-width: 0;
}

.vocab-word {
  font-weight: 600;
  font-size: 15px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--color-text);
  cursor: pointer;
  overflow-wrap: anywhere;
  text-align: left;
}

.vocab-word:hover {
  color: var(--color-accent);
}

.vocab-mastery {
  padding: 1px 8px;
  border-radius: 999px;
  border: 1px solid var(--color-border);
  background: transparent;
  font-size: 11px;
  color: var(--color-text-secondary);
  cursor: pointer;
  flex-shrink: 0;
}

.vocab-mastery.is-learning {
  border-color: #f0a020;
  color: #f0a020;
}

.vocab-mastery.is-mastered {
  border-color: #18a058;
  color: #18a058;
}

.vocab-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.vocab-translation {
  margin: 0;
  font-size: 13.5px;
  overflow-wrap: anywhere;
}

.vocab-notes {
  margin: 0;
  font-size: 12.5px;
  color: var(--color-text-secondary);
  overflow-wrap: anywhere;
}

/* 词卡：内嵌词典面板 */
.vocab-card {
  border: 1px solid var(--color-border);
  border-radius: 8px;
  background: var(--color-surface);
  padding: 8px 10px;
  max-height: 360px;
  overflow-y: auto;
}

.vocab-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 2px;
}

.vocab-meta {
  min-width: 0;
  font-size: 11.5px;
  color: var(--color-text-secondary);
}

.vocab-actions {
  display: flex;
  flex-shrink: 0;
  gap: 6px;
}

.vocab-pagination {
  display: flex;
  justify-content: center;
  margin-top: 14px;
}
</style>
