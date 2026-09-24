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
  NPopconfirm,
  NSelect,
  NSpace,
  NTag,
  NText,
  NThing,
} from "naive-ui";
import { MASTERY_OPTIONS, useVocabularyStore } from "../stores/vocabulary";
import type { Mastery, VocabularyItemDto } from "../lib/api";

const store = useVocabularyStore();
const { items, tags, loadError, editError, filter } = storeToRefs(store);

const editingId = ref<number | null>(null);
const editForm = reactive({ notes: "", tags: "", mastery: "new" as Mastery });

const masteryTagType = computed(() => (mastery: Mastery) =>
  mastery === "mastered" ? "success" : mastery === "learning" ? "warning" : "default",
);

const masteryLabel = computed(() => (mastery: Mastery) =>
  MASTERY_OPTIONS.find((option) => option.value === mastery)?.label ?? mastery,
);

const masteryOptions = MASTERY_OPTIONS.map((option) => ({
  label: option.label,
  value: option.value,
}));

const tagOptions = computed(() => tags.value.map((tag) => ({ label: tag, value: tag })));

function applyFilters(): void {
  void store.load();
}

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
        {{ items.length }} 条
      </n-text>
    </template>

    <n-text
      depth="3"
      style="display: block; margin-bottom: 12px"
    >
      划词翻译单词时自动记录（含查询次数）；Popup 里的 ★ 只针对生词，可随时移除。
      词典回答「是什么意思」，生词本记录「对你意味着什么」。
    </n-text>
    <n-alert
      v-if="loadError"
      type="error"
      :show-icon="true"
      style="margin-bottom: 12px"
    >
      {{ loadError }}
    </n-alert>

    <!-- 筛选 -->
    <n-space
      :size="12"
      style="margin-bottom: 12px"
    >
      <n-input
        v-model:value="filter.search"
        placeholder="搜索（词 / 译文 / 笔记），回车应用"
        style="flex: 2"
        clearable
        @keydown.enter="applyFilters"
        @blur="applyFilters"
      />
      <n-select
        v-model:value="filter.mastery"
        :options="[{ label: '全部掌握程度', value: '' }, ...MASTERY_OPTIONS.map((option) => ({ label: option.label, value: option.value }))]"
        placeholder="掌握程度"
        style="flex: 1"
        @update:value="applyFilters"
      />
      <n-select
        v-model:value="filter.tag"
        :options="[{ label: '全部标签', value: '' }, ...tagOptions]"
        placeholder="标签"
        style="flex: 1"
        clearable
        @update:value="applyFilters"
      />
    </n-space>

    <!-- 词条列表 -->
    <n-list
      v-if="items.length > 0"
      :show-divider="true"
    >
      <n-list-item
        v-for="item in items"
        :key="item.id"
      >
        <!-- 浏览态 -->
        <div v-if="editingId !== item.id">
          <n-thing
            content-style="display: flex; justify-content: space-between; gap: 12px; align-items: flex-start;"
          >
            <div class="row-main">
              <n-space
                :size="6"
                align="center"
                style="margin-bottom: 2px"
              >
                <n-text
                  strong
                  style="font-size: 15px"
                >
                  {{ item.word }}
                </n-text>
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
              </n-space>
              <n-text
                v-if="item.translation"
                class="row-detail"
                style="display: block; font-size: 13.5px"
              >
                {{ item.translation }}
              </n-text>
              <n-text
                v-if="item.notes"
                depth="3"
                class="row-detail"
                style="display: block"
              >
                {{ item.notes }}
              </n-text>
              <n-text
                depth="3"
                class="row-detail"
                style="display: block; font-size: 11.5px"
              >
                查询 {{ item.lookupCount }} · 复习 {{ item.reviewCount }} ·
                {{ item.lastSeenAt.slice(0, 10) }} · {{ item.source }}
              </n-text>
            </div>
            <n-space :size="6">
              <n-button
                size="small"
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
                    size="small"
                    type="error"
                    secondary
                  >
                    删除
                  </n-button>
                </template>
                从生词本移除该词条，确定？
              </n-popconfirm>
            </n-space>
          </n-thing>
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
            <n-space :size="12">
              <n-form-item
                label="标签（逗号分隔）"
                style="flex: 1"
              >
                <n-input
                  v-model:value="editForm.tags"
                  placeholder="gre, noun"
                />
              </n-form-item>
              <n-form-item
                label="掌握程度"
                style="flex: 1"
              >
                <n-select
                  v-model:value="editForm.mastery"
                  :options="masteryOptions"
                />
              </n-form-item>
            </n-space>
            <n-text
              v-if="item.examples.length > 0"
              depth="3"
              style="font-size: 12px"
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
  </n-card>
</template>
