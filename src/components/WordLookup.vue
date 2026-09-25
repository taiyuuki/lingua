<script setup lang="ts">
import { ref } from "vue";
import {
  NButton,
  NCard,
  NInput,
  NText,
} from "naive-ui";
import { lookupEntriesForWord } from "../lib/dictRender";
import type { DictionaryEntryDto } from "../lib/api";
import DictionaryEntryPanel from "./DictionaryEntryPanel.vue";

/** 内置词典查词：输入词头 → 全部命中词典的完整释义（与弹窗/词卡同源渲染）。 */
const word = ref("");
const entries = ref<DictionaryEntryDto[]>([]);
const searched = ref(false);
const searching = ref(false);

async function search(): Promise<void> {
  const query = word.value.trim();
  if (!query) {
    return;
  }
  searching.value = true;
  try {
    entries.value = await lookupEntriesForWord(query);
  } finally {
    searched.value = true;
    searching.value = false;
  }
}

/** 词卡内 entry:// 跳词：直接换查。 */
async function lookupWord(target: string): Promise<void> {
  if (!target) {
    return;
  }
  word.value = target;
  await search();
}
</script>

<template>
  <n-card
    title="查词"
    size="small"
  >
    <n-input
      v-model:value="word"
      placeholder="输入词头，回车查全部内置词典（牛津 / 日汉 / 西汉 …）"
      clearable
      @keydown.enter="search"
    >
      <template #suffix>
        <n-button
          size="tiny"
          type="primary"
          :loading="searching"
          @click="search"
        >
          查词
        </n-button>
      </template>
    </n-input>

    <div
      v-if="entries.length > 0"
      class="lookup-result"
    >
      <DictionaryEntryPanel
        :entries="entries"
        @lookup-word="lookupWord"
      />
    </div>
    <n-text
      v-else-if="searched"
      depth="3"
      style="display: block; margin-top: 10px; font-size: 12.5px"
    >
      没有词典收录「{{ word }}」；换个拼写，或在下方导入更多词典。
    </n-text>
  </n-card>
</template>

<style scoped>
.lookup-result {
  margin-top: 12px;
  padding: 10px 12px;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  background: var(--color-surface);
  max-height: 420px;
  overflow-y: auto;
}
</style>
