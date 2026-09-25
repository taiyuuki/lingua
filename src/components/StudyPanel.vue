<script setup lang="ts">
import type { StudyAnalysis } from "../lib/api";

/**
 * 学习分析卡片（Phase 9）：StudyAnalyzer 的结构化结果展示。
 * 所有块可选——按数据有无渲染；`notes` 是解析失败的降级全文。
 * 纯展示组件：加载/错误态由父级传入，关闭经 emit 上抛。
 */
defineProps<{
  analysis: StudyAnalysis | null;
  loading: boolean;
  error: string;
}>();

const emit = defineEmits<{ close: [] }>();

const kindLabel = (kind: string | null): string =>
  kind === "word" ? "单词" : kind === "sentence" ? "句子" : "";
</script>

<template>
  <div class="study-panel">
    <div class="study-header">
      <span class="result-engine">学习分析</span>
      <span
        v-if="kindLabel(analysis?.kind ?? null)"
        class="chip"
      >
        {{ kindLabel(analysis?.kind ?? null) }}
      </span>
      <span
        v-if="analysis?.phonetic"
        class="study-phonetic"
      >{{ analysis.phonetic }}</span>
      <span class="study-spacer" />
      <span
        v-if="analysis?.language"
        class="chip"
      >{{ analysis.language }}</span>
      <button
        class="tool-btn"
        type="button"
        title="关闭"
        @click="emit('close')"
      >
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
        >
          <path d="M6 6l12 12M18 6L6 18" />
        </svg>
      </button>
    </div>

    <div
      v-if="loading"
      class="study-loading"
    >
      <span
        class="spinner"
        aria-hidden="true"
      />
      分析中…
    </div>

    <p
      v-else-if="error"
      class="study-error"
    >
      {{ error }}
    </p>

    <div
      v-else-if="analysis"
      class="study-body"
    >
      <!-- 降级：模型输出无法结构化，展示原文 -->
      <p
        v-if="analysis.notes"
        class="study-notes"
      >
        {{ analysis.notes }}
      </p>

      <template v-else>
        <!-- 词块 -->
        <div
          v-if="analysis.partsOfSpeech.length > 0"
          class="study-chip-row"
        >
          <span
            v-for="pos in analysis.partsOfSpeech"
            :key="pos"
            class="chip"
          >{{ pos }}</span>
        </div>

        <ul
          v-if="analysis.senses.length > 0"
          class="study-senses"
        >
          <li
            v-for="(sense, index) in analysis.senses"
            :key="index"
          >
            <span
              v-if="sense.pos"
              class="study-pos"
            >{{ sense.pos }}</span>
            <span class="study-meaning">{{ sense.meaning }}</span>
            <p
              v-if="sense.example"
              class="study-example"
            >
              {{ sense.example }}
              <span
                v-if="sense.exampleTranslation"
                class="study-example-tr"
              >{{ sense.exampleTranslation }}</span>
            </p>
          </li>
        </ul>

        <div
          v-if="analysis.morphology"
          class="study-block"
        >
          <p
            v-if="analysis.morphology.root"
            class="study-line"
          >
            <span class="study-label">词根</span>{{ analysis.morphology.root }}
          </p>
          <p
            v-if="analysis.morphology.affixes"
            class="study-line"
          >
            <span class="study-label">词缀</span>{{ analysis.morphology.affixes }}
          </p>
          <p
            v-if="analysis.morphology.cognates"
            class="study-line"
          >
            <span class="study-label">同源</span>{{ analysis.morphology.cognates }}
          </p>
        </div>

        <div
          v-if="analysis.collocations.length > 0"
          class="study-chip-row"
        >
          <span
            v-for="collocation in analysis.collocations"
            :key="collocation"
            class="chip"
          >{{ collocation }}</span>
        </div>

        <ul
          v-if="analysis.synonyms.length > 0"
          class="study-synonyms"
        >
          <li
            v-for="synonym in analysis.synonyms"
            :key="synonym.word"
          >
            <span class="study-synonym-word">{{ synonym.word }}</span>
            <span
              v-if="synonym.nuance"
              class="study-synonym-nuance"
            >{{ synonym.nuance }}</span>
          </li>
        </ul>

        <!-- 句块 -->
        <p
          v-if="analysis.structure"
          class="study-line"
        >
          <span class="study-label">结构</span>{{ analysis.structure }}
        </p>

        <ul
          v-if="analysis.keyPhrases.length > 0"
          class="study-senses"
        >
          <li
            v-for="keyPhrase in analysis.keyPhrases"
            :key="keyPhrase.phrase"
          >
            <span class="study-meaning">{{ keyPhrase.phrase }}</span>
            <span
              v-if="keyPhrase.meaning"
              class="study-synonym-nuance"
            >{{ keyPhrase.meaning }}</span>
          </li>
        </ul>

        <p
          v-if="analysis.paraphrase"
          class="study-line"
        >
          <span class="study-label">复述</span>{{ analysis.paraphrase }}
        </p>

        <ul
          v-if="analysis.examples.length > 0"
          class="study-senses"
        >
          <li
            v-for="(example, index) in analysis.examples"
            :key="index"
          >
            <span class="study-meaning">{{ example.text }}</span>
            <span
              v-if="example.translation"
              class="study-synonym-nuance"
            >{{ example.translation }}</span>
          </li>
        </ul>

        <p
          v-if="analysis.mnemonics"
          class="study-mnemonics"
        >
          {{ analysis.mnemonics }}
        </p>
      </template>
    </div>
  </div>
</template>
