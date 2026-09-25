<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { storeToRefs } from "pinia";
import {
  NAlert,
  NButton,
  NCard,
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
import type { Mastery, VocabularyItemDto } from "../lib/api";

const store = useVocabularyStore();
const { items, total, page, tags, loadError, editError, filter } =
  storeToRefs(store);

const editingId = ref<number | null>(null);
const editForm = reactive({ notes: "", tags: "", mastery: "new" as Mastery });

const masteryTagType = (mastery: Mastery): "success" | "warning" | "default" =>
  mastery === "mastered" ? "success" : mastery === "learning" ? "warning" : "default";

const masteryLabel = (mastery: Mastery): string =>
  MASTERY_OPTIONS.find((option) => option.value === mastery)?.label ?? mastery;

const masteryOptions = MASTERY_OPTIONS.map((option) => ({
  label: option.label,
  value: option.value,
}));

const tagOptions = computed(() =>
  tags.value.map((tag) => ({ label: tag, value: tag })),
);

const pageCount = computed(() =>
  Math.max(1, Math.ceil(total.value / VOCABULARY_PAGE_SIZE)),
);

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
      划词翻译单词时自动记录（含查询次数）；Popup 里的 ★ 只针对生词，可随时移除。
    </n-text>
    <n-alert
      v-if="loadError"
      type="error"
      :show-icon="true"
      style="margin-bottom: 12px"
    >
      {{ loadError }}
    </n-alert>

    <!-- 筛选：固定三列，回车/变更即应用 -->
    <div class="vocab-filter">
      <n-input
        v-model:value="filter.search"
        placeholder="搜索（词 / 译文 / 笔记），回车应用"
        clearable
        @keydown.enter="store.applyFilters()"
        @blur="store.applyFilters()"
      />
      <n-select
        v-model:value="filter.mastery"
        :options="[{ label: '全部掌握程度', value: '' }, ...masteryOptions]"
        placeholder="掌握程度"
        @update:value="store.applyFilters()"
      />
      <n-select
        v-model:value="filter.tag"
        :options="[{ label: '全部标签', value: '' }, ...tagOptions]"
        placeholder="标签"
        clearable
        @update:value="store.applyFilters()"
      />
    </div>

    <!-- 词条：分页每页 20 条，卡片三层（头/摘要/底栏），按钮固定右下 -->
    <n-list
      v-if="items.length > 0"
      :show-divider="true"
    >
      <n-list-item
        v-for="item in items"
        :key="item.id"
      >
        <!-- 浏览态 -->
        <div
          v-if="editingId !== item.id"
          class="vocab-item"
        >
          <div class="vocab-head">
            <span class="vocab-word">{{ item.word }}</span>
            <n-tag
              size="small"
              :type="masteryTagType(item.mastery)"
              :bordered="item.mastery === 'new'"
            >
              {{ masteryLabel(item.mastery) }}
            </n-tag>
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

        <!-- 编辑态 -->
        <n-form
          v-else
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
              <n-form-item label="掌握程度">
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
/* 筛选行：固定三列（搜索占半），不随窗口随机换行 */
.vocab-filter {
  display: grid;
  grid-template-columns: 2fr 1fr 1fr;
  gap: 12px;
  margin-bottom: 12px;
}

/* 词条卡片：垂直三层，杜绝左右两栏挤压导致的按钮漂移 */
.vocab-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
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
  overflow-wrap: anywhere;
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

/* 底栏：meta 单行截断在左，操作按钮整体固定右侧、永不换行 */
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
