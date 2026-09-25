<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  dispatchRichClick,
  isLongDefinition,
  renderRichHtml,
  scrollToAnchor,
  type EntryLink,
} from "../lib/dictRender";
import type { DictionaryEntryDto } from "../lib/api";

/**
 * 词典词条面板：多词典 chips + 富文本/纯文本释义 + 发音 + entry:// 交互。
 * Popup 词典区与生词本词卡共用。词条数据由父级持有（props），跳词
 * （entry://词头）通过 `lookup-word` 事件交给父级换数据。
 */
const props = defineProps<{
  entries: DictionaryEntryDto[];
  /** 发音是否可用（词卡与弹窗都为 true；预留） */
  interactive?: boolean;
}>();

const emit = defineEmits<{
  /** entry://词头 跳词：父级查词典后更新 `entries` */
  (event: "lookup-word", word: string): void;
  /** 当前展示的词典词条变化（父级更新来源标签/收藏词头用） */
  (event: "active-change", entry: DictionaryEntryDto | null): void;
}>();

const activeIndex = ref(0);
const expandedDefinitions = ref(new Set<number>());

const activeEntry = computed(() => props.entries[activeIndex.value] ?? null);

watch(
  () => props.entries,
  () => {
    activeIndex.value = 0;
    expandedDefinitions.value = new Set();
  },
);
watch(
  activeEntry,
  (entry) => {
    emit("active-change", entry);
  },
  { immediate: true },
);

function switchDictionary(index: number): void {
  activeIndex.value = index;
  expandedDefinitions.value = new Set();
}

function toggleDefinition(index: number, text: string): void {
  if (!isLongDefinition(text)) {
    return;
  }
  const next = new Set(expandedDefinitions.value);
  if (next.has(index)) {
    next.delete(index);
  } else {
    next.add(index);
  }
  expandedDefinitions.value = next;
}

function onRichClick(event: MouseEvent): void {
  const link: EntryLink = dispatchRichClick(event);
  if (!link) {
    return;
  }
  if (link.kind === "anchor") {
    const container = (event.currentTarget as HTMLElement | null) ?? null;
    scrollToAnchor(container, link.anchor);
    return;
  }
  if (link.kind === "word" && link.word) {
    emit("lookup-word", link.word);
  }
}
</script>

<template>
  <div
    v-if="activeEntry"
    class="dict-panel"
  >
    <div
      v-if="entries.length > 1"
      class="popup-dict-switcher"
    >
      <button
        v-for="(entry, index) in entries"
        :key="entry.source"
        type="button"
        :class="['popup-dict-tab', {
          'popup-dict-tab-active': index === activeIndex,
        }]"
        :title="entry.lemma"
        @click="switchDictionary(index)"
      >
        {{ entry.source }}
      </button>
    </div>

    <ol class="popup-dict-definitions">
      <li
        v-for="(definition, index) in activeEntry.definitions.slice(0, 8)"
        :key="index"
      >
        <!-- eslint-disable vue/no-v-html -->
        <div
          v-if="definition.html"
          class="popup-dict-rich"
          @click="onRichClick($event)"
          v-html="renderRichHtml(definition.html)"
        />
        <!-- eslint-enable vue/no-v-html -->
        <div
          v-else
          :class="['popup-dict-text', {
            'popup-dict-clamped':
              isLongDefinition(definition.definition)
              && !expandedDefinitions.has(index),
          }]"
        >
          <span
            v-if="definition.partOfSpeech"
            class="chip"
          >{{ definition.partOfSpeech }}</span>
          {{ definition.definition }}
        </div>
        <span
          v-if="!definition.html && isLongDefinition(definition.definition)"
          class="popup-dict-toggle"
          @click="toggleDefinition(index, definition.definition)"
        >
          {{ expandedDefinitions.has(index) ? "收起 ▴" : "展开全部 ▾" }}
        </span>
        <ul
          v-if="definition.examples.length > 0"
          class="popup-dict-examples"
        >
          <li
            v-for="example in definition.examples.slice(0, 2)"
            :key="example"
          >
            {{ example }}
          </li>
        </ul>
      </li>
    </ol>
  </div>
</template>

<style scoped>
.dict-panel {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  user-select: text;
}
</style>
