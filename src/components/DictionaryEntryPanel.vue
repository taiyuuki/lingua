<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  dispatchRichClick,
  isLongDefinition,
  platformResourceUrl,
  playResourceAudio,
  renderRichHtml,
  scrollToAnchor,
  type EntryLink,
} from "../lib/dictRender";
import { lookupWordAudio, type DictionaryEntryDto, type WordAudioDto } from "../lib/api";

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
/** 当前词条的发音库音频（英/美，可能为空）。 */
const wordAudio = ref<WordAudioDto[]>([]);

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
    void loadWordAudio(entry);
  },
  { immediate: true },
);

/** 查当前词头的发音库音频；跳词/切换快时只认最后一次。 */
async function loadWordAudio(entry: DictionaryEntryDto | null): Promise<void> {
  const lemma = entry?.lemma.trim() ?? "";
  if (!lemma || props.interactive === false) {
    wordAudio.value = [];
    return;
  }
  try {
    const audio = await lookupWordAudio(lemma);
    if (activeEntry.value?.lemma === entry?.lemma) {
      wordAudio.value = audio;
    }
  } catch {
    wordAudio.value = [];
  }
}

function playAudio(url: string): void {
  playResourceAudio(platformResourceUrl(url));
}

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

    <!-- 词头发音（发音库命中时显示；富文本内的 sound:// 喇叭照旧独立工作） -->
    <div
      v-if="wordAudio.length > 0"
      class="dict-audio-row"
    >
      <span class="dict-audio-lemma">{{ activeEntry.lemma }}</span>
      <button
        v-for="audio in wordAudio"
        :key="audio.accent"
        type="button"
        class="dict-audio-btn"
        :title="audio.accent === 'uk' ? '英音（发音库）' : '美音（发音库）'"
        @click="playAudio(audio.url)"
      >
        🔊 {{ audio.accent === "uk" ? "英" : "美" }}
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

.dict-audio-row {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.dict-audio-lemma {
  font-weight: 600;
}

.dict-audio-btn {
  padding: 1px 8px;
  border: 1px solid var(--color-border);
  border-radius: 10px;
  background: var(--color-surface);
  color: var(--color-text);
  font-size: 12px;
  line-height: 18px;
  cursor: pointer;
}

.dict-audio-btn:hover {
  border-color: var(--color-primary);
  color: var(--color-primary);
}
</style>
