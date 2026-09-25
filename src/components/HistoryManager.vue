<script setup lang="ts">
import { onMounted, ref } from "vue";
import { storeToRefs } from "pinia";
import {
  NAlert,
  NButton,
  NCard,
  NEmpty,
  NInput,
  NList,
  NListItem,
  NPagination,
  NPopconfirm,
  NTag,
  NText,
} from "naive-ui";
import {
  HISTORY_KIND_OPTIONS,
  HISTORY_PAGE_SIZE,
  useHistoryStore,
} from "../stores/history";
import { SUPPORTED_LANGUAGES } from "../stores/settings";
import type { HistoryKind } from "../lib/api";

const store = useHistoryStore();
const { items, total, page, loadError, filter } = storeToRefs(store);

const kindLabel = (kind: HistoryKind): string =>
  HISTORY_KIND_OPTIONS.find((option) => option.value === kind)?.label ?? kind;

const languageLabel = (code: string): string =>
  SUPPORTED_LANGUAGES.find((lang) => lang.code === code)?.label ?? code;

/** ISO 时间戳 → 本地短格式（跨天带日期，当天只带时分）。 */
function formatTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return iso;
  }
  const now = new Date();
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  const hm = `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
  return sameDay ? hm : `${date.getMonth() + 1}/${date.getDate()} ${hm}`;
}

function toggleKindFilter(kind: HistoryKind): void {
  filter.value.kind = filter.value.kind === kind ? "" : kind;
  void store.applyFilters();
}

// ---- 复制译文（行内反馈） ----
const copiedId = ref<number | null>(null);
let copiedTimer: number | undefined;

async function copyTranslation(id: number, text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    copiedId.value = id;
    window.clearTimeout(copiedTimer);
    copiedTimer = window.setTimeout(() => {
      copiedId.value = null;
    }, 1500);
  } catch {
    // 剪贴板不可用时静默忽略。
  }
}

// ---- 清空全部（两段点击确认） ----
const confirmingClear = ref(false);

async function onClearAll(): Promise<void> {
  await store.clear();
  confirmingClear.value = false;
}

onMounted(() => {
  void store.load();
});
</script>

<template>
  <n-card
    title="翻译历史"
    size="small"
  >
    <template #header-extra>
      <n-text
        depth="3"
        style="font-size: 12px"
      >
        共 {{ total }} 条 · 只保留最近 5000 条
      </n-text>
    </template>

    <n-alert
      v-if="loadError"
      type="error"
      :show-icon="true"
      style="margin-bottom: 12px"
    >
      {{ loadError }}
    </n-alert>

    <div class="history-toolbar">
      <n-input
        v-model:value="filter.search"
        size="small"
        clearable
        placeholder="搜索原文或译文…"
        style="flex: 1"
        @update:value="store.applyFilters()"
      />
      <n-button
        size="small"
        :type="filter.kind === '' ? 'primary' : 'default'"
        secondary
        @click="filter.kind = ''; store.applyFilters()"
      >
        全部
      </n-button>
      <n-button
        v-for="option in HISTORY_KIND_OPTIONS"
        :key="option.value"
        size="small"
        :type="filter.kind === option.value ? 'primary' : 'default'"
        secondary
        @click="toggleKindFilter(option.value)"
      >
        {{ option.label }}
      </n-button>
      <n-popconfirm
        :show-icon="false"
        @positive-click="onClearAll"
      >
        <template #trigger>
          <n-button
            size="small"
            type="error"
            secondary
            :disabled="total === 0"
          >
            清空
          </n-button>
        </template>
        删除全部 {{ total }} 条历史，确定？
      </n-popconfirm>
    </div>

    <n-empty
      v-if="items.length === 0"
      description="还没有翻译记录。划词或截图翻译后会自动出现在这里。"
      style="padding: 32px 0"
    />

    <n-list
      v-else
      :show-divider="true"
    >
      <n-list-item
        v-for="item in items"
        :key="item.id"
      >
        <div class="history-item">
          <div class="history-head">
            <n-tag
              size="small"
              :bordered="false"
              :type="item.kind === 'ocr' ? 'warning' : 'default'"
            >
              {{ kindLabel(item.kind) }}
            </n-tag>
            <span
              v-if="item.detectedLanguage"
              class="history-lang"
            >
              {{ languageLabel(item.detectedLanguage) }} →
              {{ languageLabel(item.targetLanguage) }}
            </span>
            <span
              v-else
              class="history-lang"
            >→ {{ languageLabel(item.targetLanguage) }}</span>
            <span class="history-spacer" />
            <span
              class="history-engine"
              :title="item.dictionaryHit ? '词典命中' : (item.model ?? '')"
            >
              {{ item.dictionaryHit ? "词典" : (item.model ?? "AI") }}
            </span>
            <span class="history-time">{{ formatTime(item.createdAt) }}</span>
          </div>

          <p
            class="history-source clamp-2"
            :title="item.sourceText"
          >
            {{ item.sourceText }}
          </p>
          <p
            class="history-translation clamp-2"
            :title="item.translatedText"
          >
            {{ item.translatedText }}
          </p>

          <div class="history-foot">
            <span class="history-spacer" />
            <n-button
              size="tiny"
              secondary
              @click="copyTranslation(item.id, item.translatedText)"
            >
              {{ copiedId === item.id ? "已复制" : "复制译文" }}
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
              删除这条历史，确定？
            </n-popconfirm>
          </div>
        </div>
      </n-list-item>
    </n-list>

    <div
      v-if="total > HISTORY_PAGE_SIZE"
      class="history-pagination"
    >
      <n-pagination
        :page="page"
        :page-count="Math.max(1, Math.ceil(total / HISTORY_PAGE_SIZE))"
        @update:page="store.turnTo($event)"
      />
    </div>
  </n-card>
</template>

<style scoped>
.history-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
}

.history-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
  user-select: text;
}

.history-item :deep(button) {
  user-select: none;
}

.history-head {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  min-width: 0;
}

.history-lang {
  font-size: 11.5px;
  color: var(--color-text-secondary);
}

.history-spacer {
  flex: 1;
}

.history-engine {
  font-size: 11px;
  color: var(--color-text-secondary);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: 4px;
  padding: 0 5px;
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.history-time {
  font-size: 11.5px;
  color: var(--color-text-secondary);
  flex-shrink: 0;
}

.history-source {
  margin: 0;
  font-size: 12.5px;
  color: var(--color-text-secondary);
  overflow-wrap: anywhere;
}

.history-translation {
  margin: 0;
  font-size: 13.5px;
  overflow-wrap: anywhere;
}

.history-foot {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  margin-top: 2px;
}

.history-pagination {
  display: flex;
  justify-content: center;
  margin-top: 14px;
}
</style>
