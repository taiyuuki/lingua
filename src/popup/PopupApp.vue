<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  hidePopup,
  latestPopupState,
  onPopupState,
  saveToVocabulary,
  togglePopupPin,
  type PopupState,
} from "../lib/api";

// 快照同款「事件 + 快照」双通道；窗口复用，靠通知保持最新状态。
// 布局参考 Bob：顶部工具栏（置顶/收藏/复制原文）→ 原文区 → 译文卡片。
const state = ref<PopupState>({ status: "loading" });
const copiedSource = ref(false);
const copiedTranslation = ref(false);
const saved = ref(false);
const pinned = ref(true);
let unlisten: (() => void) | null = null;

const engineLabel = computed(() =>
  state.value.status === "result" ? (state.value.dictionary?.source ?? "AI 翻译") : "",
);
const detectedLanguageLabel = computed(() =>
  state.value.status === "result" ? state.value.detectedLanguage : null,
);

async function copyText(text: string, which: "source" | "translation"): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    if (which === "source") {
      copiedSource.value = true;
      window.setTimeout(() => {
        copiedSource.value = false;
      }, 1500);
    } else {
      copiedTranslation.value = true;
      window.setTimeout(() => {
        copiedTranslation.value = false;
      }, 1500);
    }
  } catch {
    // 剪贴板不可用时静默忽略。
  }
}

async function togglePin(): Promise<void> {
  try {
    pinned.value = await togglePopupPin();
  } catch {
    // 切换失败保持原状态。
  }
}

async function saveToVocabularyBook(): Promise<void> {
  if (state.value.status !== "result" || saved.value) {
    return;
  }
  try {
    await saveToVocabulary({
      word: state.value.sourceText.trim(),
      language: state.value.detectedLanguage,
      translation: state.value.translatedText,
      source: state.value.dictionary?.source ?? "manual",
      examples: state.value.dictionary?.definitions.flatMap((d) => d.examples).slice(0, 4) ?? [],
    });
    saved.value = true;
  } catch {
    // 收藏失败静默：主流程不受影响。
  }
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === "Escape") {
    void hidePopup();
  }
}

onMounted(async () => {
  window.addEventListener("keydown", onKeydown);
  const stop = await onPopupState((next) => {
    state.value = next;
    copiedSource.value = false;
    copiedTranslation.value = false;
    saved.value = false;
  });
  unlisten = stop;
  const snapshot = await latestPopupState();
  if (snapshot) {
    state.value = snapshot;
  }
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKeydown);
  unlisten?.();
});
</script>

<template>
  <div class="popup-root">
    <!-- 工具栏：置顶 | 收藏 / 复制原文 -->
    <div class="popup-toolbar">
      <button
        class="tool-btn"
        :class="{ 'tool-active': pinned }"
        type="button"
        title="置顶"
        @click="togglePin"
      >
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M12 21s-6-5.2-6-10a6 6 0 1 1 12 0c0 4.8-6 10-6 10z" />
          <circle
            cx="12"
            cy="11"
            r="2.2"
          />
        </svg>
      </button>

      <div class="tool-spacer" />

      <template v-if="state.status === 'result'">
        <button
          class="tool-btn"
          :class="{ 'tool-active': saved }"
          type="button"
          title="加入生词本"
          :disabled="saved"
          @click="saveToVocabularyBook"
        >
          <svg
            viewBox="0 0 24 24"
            :fill="saved ? 'currentColor' : 'none'"
            stroke="currentColor"
            stroke-width="1.8"
            stroke-linejoin="round"
          >
            <path d="M12 3l2.7 5.6 6.1.8-4.5 4.2 1.1 6-5.4-3-5.4 3 1.1-6L3.2 9.4l6.1-.8z" />
          </svg>
        </button>
        <button
          class="tool-btn"
          type="button"
          title="复制原文"
          @click="copyText(state.sourceText, 'source')"
        >
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.8"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <rect
              x="9"
              y="9"
              width="11"
              height="11"
              rx="2"
            />
            <path d="M5 15V5a2 2 0 0 1 2-2h10" />
          </svg>
        </button>
      </template>
    </div>

    <!-- 加载中 -->
    <div
      v-if="state.status === 'loading'"
      class="popup-loading"
    >
      <span
        class="spinner"
        aria-hidden="true"
      />
      <span>正在翻译…</span>
    </div>

    <!-- 翻译结果 -->
    <div
      v-else-if="state.status === 'result'"
      class="popup-scroll"
    >
      <p class="popup-source">
        {{ state.sourceText }}
      </p>
      <div class="popup-source-meta">
        <button
          class="tool-btn"
          type="button"
          :title="copiedSource ? '已复制' : '复制原文'"
          @click="copyText(state.sourceText, 'source')"
        >
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.8"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <rect
              x="9"
              y="9"
              width="11"
              height="11"
              rx="2"
            />
            <path d="M5 15V5a2 2 0 0 1 2-2h10" />
          </svg>
        </button>
        <span
          v-if="detectedLanguageLabel"
          class="chip"
        >识别为 {{ detectedLanguageLabel }}</span>
      </div>

      <!-- 译文卡片 -->
      <div class="result-card">
        <div class="result-card-header">
          <span class="result-engine">{{ engineLabel }}</span>
          <button
            class="tool-btn"
            type="button"
            :title="copiedTranslation ? '已复制' : '复制译文'"
            @click="copyText(state.translatedText, 'translation')"
          >
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <rect
                x="9"
                y="9"
                width="11"
                height="11"
                rx="2"
              />
              <path d="M5 15V5a2 2 0 0 1 2-2h10" />
            </svg>
          </button>
        </div>
        <p class="popup-translation">
          {{ state.translatedText }}
        </p>

        <!-- 词典视图：词条释义/例句 -->
        <div
          v-if="state.dictionary"
          class="popup-dictionary"
        >
          <ol class="popup-dict-definitions">
            <li
              v-for="(definition, index) in state.dictionary.definitions.slice(0, 8)"
              :key="index"
            >
              <span
                v-if="definition.partOfSpeech"
                class="chip"
              >{{ definition.partOfSpeech }}</span>
              <span class="popup-dict-text">{{ definition.definition }}</span>
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

        <template v-else>
          <ul
            v-if="state.alternatives.length > 0"
            class="popup-alternatives"
          >
            <li
              v-for="alternative in state.alternatives"
              :key="alternative"
            >
              {{ alternative }}
            </li>
          </ul>
        </template>
      </div>

      <p
        v-if="state.explanation"
        class="popup-explanation"
      >
        {{ state.explanation }}
      </p>
    </div>

    <!-- 错误 -->
    <div
      v-else
      class="popup-error"
    >
      <p class="popup-error-title">
        {{ state.kind === "no_selection" ? "未选中文本" : "翻译失败" }}
      </p>
      <p class="popup-error-message">
        {{ state.message }}
      </p>
      <p
        v-if="state.kind === 'permission'"
        class="popup-error-hint"
      >
        macOS 需要在「系统设置 → 隐私与安全性」中授权本应用。
      </p>
    </div>

    <!-- 底栏 -->
    <div class="popup-footer">
      <span class="popup-brand">Lingua</span>
      <button
        class="tool-btn"
        type="button"
        title="关闭 (Esc)"
        @click="hidePopup()"
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
  </div>
</template>
