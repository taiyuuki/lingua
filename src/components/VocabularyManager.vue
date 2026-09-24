<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";
import { storeToRefs } from "pinia";
import {
  MASTERY_OPTIONS,
  useVocabularyStore,
} from "../stores/vocabulary";
import type { Mastery, VocabularyItemDto } from "../lib/api";

const store = useVocabularyStore();
const { items, tags, loadError, editError, filter } = storeToRefs(store);

const editingId = ref<number | null>(null);
const editForm = reactive({ notes: "", tags: "", mastery: "new" as Mastery });

function masteryLabel(mastery: Mastery): string {
  return MASTERY_OPTIONS.find((option) => option.value === mastery)?.label ?? mastery;
}

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
  <section class="card">
    <div class="card-header-row">
      <h2>生词本</h2>
      <span class="muted count-hint">{{ items.length }} 条</span>
    </div>
    <p class="muted">
      划词翻译单词时自动记录（含查询次数）；短语/句子可在 Popup
      里手动收藏。词典回答「是什么意思」，生词本记录「对你意味着什么」。
    </p>
    <p
      v-if="loadError"
      class="error-banner"
    >
      {{ loadError }}
    </p>

    <!-- 筛选 -->
    <div class="field-row vocab-filters">
      <label class="field">
        <span>搜索（词 / 译文 / 笔记）</span>
        <input
          v-model="filter.search"
          type="text"
          placeholder="输入后回车"
          spellcheck="false"
          @keydown.enter="applyFilters"
          @blur="applyFilters"
        >
      </label>
      <label class="field">
        <span>掌握程度</span>
        <select
          v-model="filter.mastery"
          @change="applyFilters"
        >
          <option value="">
            全部
          </option>
          <option
            v-for="option in MASTERY_OPTIONS"
            :key="option.value"
            :value="option.value"
          >
            {{ option.label }}
          </option>
        </select>
      </label>
      <label class="field">
        <span>标签</span>
        <select
          v-model="filter.tag"
          @change="applyFilters"
        >
          <option value="">
            全部
          </option>
          <option
            v-for="tag in tags"
            :key="tag"
            :value="tag"
          >
            {{ tag }}
          </option>
        </select>
      </label>
    </div>

    <!-- 列表 -->
    <ul
      v-if="items.length > 0"
      class="provider-list"
    >
      <li
        v-for="item in items"
        :key="item.id"
        class="provider-row vocab-row"
      >
        <template v-if="editingId !== item.id">
          <div class="provider-info">
            <div class="provider-title">
              <strong>{{ item.word }}</strong>
              <span
                class="chip"
                :class="`mastery-${item.mastery}`"
              >{{ masteryLabel(item.mastery) }}</span>
              <span
                v-for="tag in item.tags"
                :key="tag"
                class="chip"
              >{{ tag }}</span>
            </div>
            <p
              v-if="item.translation"
              class="provider-detail"
            >
              {{ item.translation }}
            </p>
            <p class="provider-detail">
              查询 {{ item.lookupCount }} 次 · 复习 {{ item.reviewCount }} 次 ·
              最近 {{ item.lastSeenAt.slice(0, 10) }} · 来源 {{ item.source }}
            </p>
            <p
              v-if="item.notes"
              class="provider-detail"
            >
              📝 {{ item.notes }}
            </p>
          </div>
          <div class="provider-actions">
            <button
              class="btn btn-small btn-secondary"
              type="button"
              @click="startEdit(item)"
            >
              编辑
            </button>
            <button
              class="btn btn-small btn-danger"
              type="button"
              @click="store.remove(item.id)"
            >
              删除
            </button>
          </div>
        </template>

        <!-- 编辑态 -->
        <form
          v-else
          class="settings-form vocab-edit"
          @submit.prevent="submitEdit(item.id)"
        >
          <p class="vocab-edit-word">
            {{ item.word }}
          </p>
          <label class="field">
            <span>笔记</span>
            <textarea
              v-model="editForm.notes"
              rows="2"
              placeholder="这个词对你的意义、联想、易错点…"
            />
          </label>
          <div class="field-row">
            <label class="field">
              <span>标签（逗号分隔）</span>
              <input
                v-model="editForm.tags"
                type="text"
                placeholder="gre, noun"
                spellcheck="false"
              >
            </label>
            <label class="field">
              <span>掌握程度</span>
              <select v-model="editForm.mastery">
                <option
                  v-for="option in MASTERY_OPTIONS"
                  :key="option.value"
                  :value="option.value"
                >
                  {{ option.label }}
                </option>
              </select>
            </label>
          </div>
          <p
            v-if="item.examples.length > 0"
            class="provider-detail"
          >
            例句：{{ item.examples.join(" / ") }}
          </p>
          <div class="form-footer">
            <button
              class="btn btn-small"
              type="submit"
            >
              保存
            </button>
            <button
              class="btn btn-small btn-secondary"
              type="button"
              @click="editingId = null"
            >
              取消
            </button>
          </div>
          <p
            v-if="editError"
            class="error-banner"
          >
            {{ editError }}
          </p>
        </form>
      </li>
    </ul>
    <p
      v-else-if="!loadError"
      class="muted"
    >
      还没有生词记录。用 alt+t 翻译一个单词试试。
    </p>
  </section>
</template>
