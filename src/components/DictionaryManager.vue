<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { storeToRefs } from "pinia";
import Sortable from "sortablejs";
import {
  NAlert,
  NButton,
  NCard,
  NEmpty,
  NPopconfirm,
  NSpin,
  NText,
} from "naive-ui";
import { useDictionariesStore } from "../stores/dictionaries";
import { pickAudioLibraryPath } from "../lib/api";

/** 词典列表卡片（词典管理页）：拖拽排序 + 删除 + 后台导入进度横条。 */
const store = useDictionariesStore();
const {
  dictionaries,
  loading,
  loadError,
  importing,
  cancelling,
  importProgress,
  deletingId,
  form,
  audioLibraries,
  audioLoading,
  audioImporting,
  audioError,
  audioMessage,
  audioDeletingId,
} = storeToRefs(store);

const router = useRouter();

/** 导入中横条显示的词典名。 */
const importName = computed(() => form.value.name || form.value.path || "词典");

/** 发音库口音标签（uk → 英 / us → 美）。 */
function accentLabel(accent: string): string {
  return accent === "uk" ? "英音" : "美音";
}

async function addAudioLibrary(): Promise<void> {
  const path = await pickAudioLibraryPath();
  if (!path) {
    return;
  }
  await store.importAudio(path);
}

const listEl = ref<HTMLElement | null>(null);
let sortable: Sortable | null = null;

onMounted(() => {
  void store.load();
  void store.loadAudioLibraries();
  if (listEl.value) {
    sortable = new Sortable(listEl.value, {
      handle: ".dict-drag-handle",
      animation: 150,
      // Tauri 窗口级的文件拖放拦截会吞掉 WebView 的 HTML5 拖拽事件，
      // 必须启用鼠标事件兜底模式，拖动才能在桌面端生效。
      forceFallback: true,
      fallbackOnBody: true,
      fallbackClass: "dict-row-dragging",
      fallbackTolerance: 3,
      onEnd(event) {
        const { oldIndex, newIndex, item, from } = event;
        if (oldIndex == null || newIndex == null || oldIndex === newIndex) {
          return;
        }
        // Sortable 直接搬动了 DOM：先把节点放回原位再改数据，
        // 让 Vue 按 v-for 的数据顺序接管渲染，避免 DOM 与 vdom 失配。
        const anchor = (newIndex > oldIndex
          ? from.children[oldIndex]
          : from.children[oldIndex + 1]) ?? null;
        from.insertBefore(item, anchor);
        const next = [...dictionaries.value];
        const [moved] = next.splice(oldIndex, 1);
        if (!moved) {
          return;
        }
        next.splice(newIndex, 0, moved);
        void store.applyOrder(next);
      },
    });
  }
});

onBeforeUnmount(() => {
  sortable?.destroy();
  sortable = null;
});
</script>

<template>
  <n-card
    title="词典列表"
    size="small"
  >
    <template #header-extra>
      <n-button
        size="small"
        type="primary"
        @click="router.push({ name: 'dictionary-import' })"
      >
        导入词典
      </n-button>
    </template>

    <n-text
      v-if="!loading"
      depth="3"
      style="display: block; margin-bottom: 12px"
    >
      拖动行首手柄调整顺序；查词与划词弹窗按此顺序依次命中释义。
    </n-text>
    <n-alert
      v-if="loadError"
      type="error"
      :show-icon="true"
      style="margin-bottom: 12px"
    >
      {{ loadError }}
    </n-alert>

    <!-- 首次加载动画 -->
    <div
      v-if="loading"
      class="dict-loading"
    >
      <n-spin size="small" />
      <n-text depth="3">
        正在加载词典列表…
      </n-text>
    </div>

    <!-- 后台导入中横条（导入页点「后台继续，返回列表」后仍可见） -->
    <n-alert
      v-if="importing && !loading"
      type="info"
      :show-icon="false"
      style="margin-bottom: 12px"
    >
      <div class="import-banner">
        <span>
          正在导入「{{ importName }}」
          <template v-if="importProgress">
            {{
              importProgress.phase === "parsing"
                ? "（解析索引…）"
                : `（${importProgress.imported} / ${importProgress.total} 词条）`
            }}
          </template>
        </span>
        <n-button
          size="tiny"
          type="error"
          secondary
          :loading="cancelling"
          @click="store.cancelImport()"
        >
          取消导入
        </n-button>
      </div>
    </n-alert>

    <!-- 词典列表：手柄拖拽调序（查询命中顺序随之变化）。
         容器常驻渲染（Sortable 在 onMounted 绑定一次，行由 v-for 动态填充）。 -->
    <div
      ref="listEl"
      class="dict-list"
    >
      <div
        v-for="dictionary in dictionaries"
        :key="dictionary.id"
        class="dict-row"
      >
        <div
          class="dict-drag-handle"
          title="拖动调整顺序（查询时优先命中）"
        >
          <svg
            viewBox="0 0 10 16"
            fill="currentColor"
            aria-hidden="true"
          >
            <circle
              cx="3"
              cy="3"
              r="1.5"
            />
            <circle
              cx="7"
              cy="3"
              r="1.5"
            />
            <circle
              cx="3"
              cy="8"
              r="1.5"
            />
            <circle
              cx="7"
              cy="8"
              r="1.5"
            />
            <circle
              cx="3"
              cy="13"
              r="1.5"
            />
            <circle
              cx="7"
              cy="13"
              r="1.5"
            />
          </svg>
        </div>
        <div class="dict-row-main">
          <n-text strong>
            {{ dictionary.name }}
          </n-text>
          <n-text
            depth="3"
            class="dict-row-detail"
          >
            {{ dictionary.sourceLanguage }} → {{ dictionary.glossLanguage }} ·
            {{ dictionary.entryCount }} 个词条
          </n-text>
        </div>
        <n-popconfirm
          :show-icon="false"
          :disabled="deletingId !== null"
          @positive-click="store.remove(dictionary.id)"
        >
          <template #trigger>
            <n-button
              size="small"
              type="error"
              secondary
              :loading="deletingId === dictionary.id"
              :disabled="deletingId !== null && deletingId !== dictionary.id"
            >
              删除
            </n-button>
          </template>
          全部词条数据将被移除（大词典需要几秒），确定？
        </n-popconfirm>
      </div>
    </div>
    <n-empty
      v-if="dictionaries.length === 0 && !loading"
      description="还没有词典。支持导入 TSV / MDX 词典文件（如 CC-CEDICT、ECDICT）。"
      style="margin: 12px 0"
    />
  </n-card>

  <!-- 发音库：独立 .mdd 音频包（按词头命名），任何词典查词都能出声 -->
  <n-card
    title="发音库"
    size="small"
    style="margin-top: 16px"
  >
    <template #header-extra>
      <n-button
        size="small"
        type="primary"
        :loading="audioImporting"
        @click="addAudioLibrary"
      >
        添加发音库
      </n-button>
    </template>

    <n-text
      v-if="!audioLoading"
      depth="3"
      style="display: block; margin-bottom: 12px"
    >
      独立的 .mdd 音频包（如英式美式发音库）：按词头命名音频，查词与划词弹窗里点喇叭即可播放对应口音，覆盖所有词典。
    </n-text>
    <n-alert
      v-if="audioError"
      type="error"
      :show-icon="false"
      style="margin-bottom: 12px"
    >
      {{ audioError }}
    </n-alert>
    <n-alert
      v-if="audioMessage"
      type="success"
      :show-icon="false"
      style="margin-bottom: 12px"
    >
      {{ audioMessage }}
    </n-alert>

    <div
      v-if="audioLoading"
      class="dict-loading"
    >
      <n-spin size="small" />
      <n-text depth="3">
        正在加载发音库列表…
      </n-text>
    </div>

    <div
      v-for="library in audioLibraries"
      :key="library.id"
      class="audio-row"
    >
      <div class="dict-row-main">
        <n-text strong>
          {{ library.name }}
        </n-text>
        <n-text
          depth="3"
          class="dict-row-detail"
        >
          {{ library.entryCount }} 个词头 ·
          <span
            v-for="accent in library.accents"
            :key="accent"
            class="accent-chip"
          >{{ accentLabel(accent) }}</span>
        </n-text>
      </div>
      <n-popconfirm
        :show-icon="false"
        :disabled="audioDeletingId !== null"
        @positive-click="store.removeAudio(library.id)"
      >
        <template #trigger>
          <n-button
            size="small"
            type="error"
            secondary
            :loading="audioDeletingId === library.id"
            :disabled="audioDeletingId !== null && audioDeletingId !== library.id"
          >
            删除
          </n-button>
        </template>
        将删除词头索引与收编的音频文件（立即释放磁盘），确定？
      </n-popconfirm>
    </div>
    <n-empty
      v-if="audioLibraries.length === 0 && !audioLoading && !audioImporting"
      description="还没有发音库。支持导入按词头命名的 .mdd（word.spx / us\\word.mp3 这类）。"
      style="margin: 12px 0"
    />
  </n-card>
</template>

<style scoped>
.dict-list {
  display: flex;
  flex-direction: column;
}

.dict-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 4px;
}

.dict-row + .dict-row {
  border-top: 1px solid var(--color-border);
}

.dict-drag-handle {
  display: flex;
  align-items: center;
  justify-content: center;
  flex: none;
  width: 20px;
  height: 28px;
  border-radius: 4px;
  color: var(--color-text-secondary);
  cursor: grab;
  touch-action: none;
}

.dict-drag-handle:active {
  cursor: grabbing;
}

.dict-drag-handle svg {
  width: 10px;
  height: 16px;
}

.dict-row-main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: 1;
  min-width: 0;
}

.dict-row-detail {
  font-size: 12.5px;
}

/* 拖拽跟手的浮层（sortablejs 兜底模式 clone 出的行，挂在 body 下） */
.dict-row-dragging {
  background: var(--color-surface);
  opacity: 0.95;
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.18);
}

.dict-loading {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 28px 0;
}

.import-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.audio-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 4px;
}

.audio-row + .audio-row {
  border-top: 1px solid var(--color-border);
}

.accent-chip {
  display: inline-block;
  margin-left: 4px;
  padding: 0 6px;
  border: 1px solid var(--color-border);
  border-radius: 4px;
  font-size: 11px;
  line-height: 18px;
}
</style>
