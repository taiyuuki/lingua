<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  analyzeStudy,
  getAppSettings,
  hidePopup,
  isWord,
  latestPopupState,
  latestStreamed,
  ocrRetryAvailable,
  ocrSourceInfo,
  onPopupDelta,
  onPopupState,
  onThemeChanged,
  removeVocabularyWord,
  retryOcrWithVision,
  saveToVocabulary,
  togglePopupPin,
  vocabularyWordStatus,
  type DictionaryEntryDto,
  type OcrSourceInfo,
  type PopupState,
  type StudyAnalysis,
} from "../lib/api";
import { applyTheme, normalizeTheme } from "../lib/theme";
import { lookupEntriesForWord } from "../lib/dictRender";
import DictionaryEntryPanel from "../components/DictionaryEntryPanel.vue";
import StudyPanel from "../components/StudyPanel.vue";

// 快照同款「事件 + 快照」双通道；窗口复用，靠通知保持最新状态。
// 布局参考 Bob：顶部工具栏（置顶/收藏/复制原文）→ 原文区 → 译文卡片。
const state = ref<PopupState>({ status: "loading", sourceText: null });
// 流式翻译：Loading 期间增量到达的译文（每次都是完整前缀，直接替换）
const streamed = reactive({ text: "" });
const copiedSource = ref(false);
const copiedTranslation = ref(false);
// 图钉 = 点击外部不隐藏（置顶常开，与图钉无关）；默认不固定。
const pinned = ref(false);
// 收藏：仅单词可收藏；星标是双向开关（在生词本中 ⇄ 移除）。
const starVisible = ref(false);
const saved = ref(false);
const favoriteWord = reactive({ word: "", language: null as string | null });
// 「用 Vision 重新识别」：仅截图翻译的结果且配置了视觉 Provider 时可用。
const visionRetryAvailable = ref(false);
// 识别来源：让用户能区分「识别错了」和「Vision 根本没触发」。
const ocrSource = ref<OcrSourceInfo | null>(null);
// 当前展示的词典词条（由共享面板经 active-change 上报）。
const activeEntry = ref<DictionaryEntryDto | null>(null);
// 学习分析（Phase 9）：打开时以整面板替换内容区；Esc 先关面板再关弹窗。
const study = reactive({
  visible: false,
  loading: false,
  error: "",
  data: null as StudyAnalysis | null,
});

/** 发起学习分析：单词用词典词头，其余用原文；带检测结果与译文。 */
async function runStudy(): Promise<void> {
  if (state.value.status !== "result" || study.loading) {
    return;
  }
  const text = (activeEntry.value?.lemma ?? state.value.sourceText).trim();
  if (!text) {
    return;
  }
  study.visible = true;
  study.loading = true;
  study.error = "";
  study.data = null;
  try {
    study.data = await analyzeStudy({
      text,
      language: state.value.detectedLanguage,
      translation: state.value.translatedText,
    });
  } catch (error) {
    study.error = String(error);
  } finally {
    study.loading = false;
  }
}

function closeStudy(): void {
  study.visible = false;
  study.loading = false;
  study.error = "";
  study.data = null;
}

function onActiveEntryChange(entry: DictionaryEntryDto | null): void {
  activeEntry.value = entry;
}

/** 内链跳词（面板 lookup-word 事件）：命中则替换全部词典词条，
 *  未命中/失败静默保留。 */
async function jumpToEntry(word: string): Promise<void> {
  if (!word || state.value.status !== "result") {
    return;
  }
  const entries = await lookupEntriesForWord(word);
  if (entries.length > 0 && state.value.status === "result") {
    state.value.dictionaries = entries;
  }
}
let unlisten: (() => void) | null = null;
let unlistenTheme: (() => void) | null = null;
let unlistenDelta: (() => void) | null = null;

const engineLabel = computed(() =>
  state.value.status === "result"
    ? (activeEntry.value?.source ?? state.value.model ?? "AI 翻译")
    : "",
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

async function toggleSaveToVocabulary(): Promise<void> {
  if (state.value.status !== "result" || !starVisible.value) {
    return;
  }
  try {
    if (saved.value) {
      await removeVocabularyWord(favoriteWord.word, favoriteWord.language);
      saved.value = false;
    } else {
      await saveToVocabulary({
        word: favoriteWord.word,
        language: favoriteWord.language,
        translation: state.value.translatedText,
        source: activeEntry.value?.source ?? "manual",
        examples:
          activeEntry.value?.definitions.flatMap((d) => d.examples).slice(0, 4) ?? [],
      });
      saved.value = true;
    }
  } catch {
    // 收藏操作失败静默：主流程不受影响。
  }
}

/** 评估收藏能力与初始状态：只有「单词」可收藏，结合词典词条数据。 */
async function evaluateFavorite(): Promise<void> {
  if (state.value.status !== "result") {
    starVisible.value = false;
    return;
  }
  const word = (activeEntry.value?.lemma ?? state.value.sourceText).trim();
  const language = state.value.detectedLanguage;
  try {
    starVisible.value = await isWord(word);
    if (starVisible.value) {
      favoriteWord.word = word;
      favoriteWord.language = language;
      saved.value = await vocabularyWordStatus(word, language);
    }
  } catch {
    starVisible.value = false;
  }
}

/** 刷新「用 Vision 重新识别」可用性（截图翻译保留的截图 + 视觉 Provider）。 */
async function refreshVisionRetry(): Promise<void> {
  try {
    visionRetryAvailable.value = await ocrRetryAvailable();
  } catch {
    visionRetryAvailable.value = false;
  }
}

/** 刷新识别来源标签（划词翻译为 null，不显示）。 */
async function refreshOcrSource(): Promise<void> {
  try {
    ocrSource.value = await ocrSourceInfo();
  } catch {
    ocrSource.value = null;
  }
}

const ocrSourceLabel = computed(() => {
  const source = ocrSource.value;
  if (!source) {
    return "";
  }
  const parts: string[] = [];
  if (source.engine.startsWith("vision:")) {
    parts.push(
      source.via === "manual" ? "AI 重新识别" : "AI 自动识别",
    );
  } else {
    parts.push(source.engine);
    if (source.confidence !== null) {
      parts.push(`置信度 ${(source.confidence * 100).toFixed(0)}%`);
    }
    if (source.visionNote) {
      parts.push(source.visionNote);
    }
  }
  return `识别：${parts.join(" · ")}`;
});

async function retryWithVision(): Promise<void> {
  // 失败以 popup 错误态呈现，这里静默。
  try {
    await retryOcrWithVision();
  } catch {
    // ignore
  }
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === "Escape") {
    // 学习面板打开时 Esc 只关面板，再按一次才关弹窗。
    if (study.visible) {
      closeStudy();
    } else {
      void hidePopup();
    }
  }
}

onMounted(async () => {
  window.addEventListener("keydown", onKeydown);
  const stopTheme = await onThemeChanged((theme) => {
    applyTheme(theme);
  });
  unlistenTheme = stopTheme;
  try {
    applyTheme(normalizeTheme((await getAppSettings()).uiTheme));
  } catch {
    applyTheme("dark");
  }
  const stopDelta = await onPopupDelta((text) => {
    streamed.text = text;
  });
  const stop = await onPopupState((next) => {
    state.value = next;
    // 新一次查询：清掉上次的词典词条（词典视图卸载时不会发
    // active-change，不重置会让 AI 结果顶着急用旧词典名当标题）。
    activeEntry.value = null;
    streamed.text = "";
    copiedSource.value = false;
    copiedTranslation.value = false;
    closeStudy();
    void evaluateFavorite();
    void refreshVisionRetry();
    void refreshOcrSource();
  });
  unlisten = stop;
  unlistenDelta = stopDelta;
  const snapshot = await latestPopupState();
  if (snapshot) {
    state.value = snapshot;
    activeEntry.value = null;
  }
  // 追平：挂载期间可能已漏掉若干 delta 事件
  streamed.text = await latestStreamed();
  void evaluateFavorite();
  void refreshVisionRetry();
  void refreshOcrSource();
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKeydown);
  unlisten?.();
  unlistenTheme?.();
  unlistenDelta?.();
});

// 无边框窗口拖动：工具栏按下（按钮除外）即开始拖拽。
function onToolbarMouseDown(event: MouseEvent): void {
  if (event.button !== 0) {
    return;
  }
  const target = event.target as HTMLElement | null;
  if (target?.closest("button")) {
    return;
  }
  void getCurrentWindow().startDragging();
}
</script>

<template>
  <div class="popup-root">
    <!-- 工具栏：置顶 | 收藏 / 复制原文 -->
    <div
      class="popup-toolbar"
      @mousedown="onToolbarMouseDown"
    >
      <button
        class="tool-btn"
        :class="{ 'tool-active': pinned }"
        type="button"
        title="固定：点击外部不隐藏"
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
          type="button"
          title="AI 学习分析：词根词缀 / 例句搭配 / 语法解构"
          @click="runStudy"
        >
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.8"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M12 3l9 4.5-9 4.5-9-4.5z" />
            <path d="M5.5 10v4.6c0 1.5 2.9 3.4 6.5 3.4s6.5-1.9 6.5-3.4V10" />
            <path d="M21 7.5V13" />
          </svg>
        </button>
        <button
          v-if="visionRetryAvailable"
          class="tool-btn"
          type="button"
          title="识别不准确时，用 AI 重新识别"
          @click="retryWithVision"
        >
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.8"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
            <circle
              cx="12"
              cy="12"
              r="3"
            />
          </svg>
        </button>
        <button
          v-if="starVisible"
          class="tool-btn"
          :class="{ 'tool-active': saved }"
          type="button"
          :title="saved ? '从生词本移除' : '加入生词本'"
          @click="toggleSaveToVocabulary"
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

    <!-- 学习分析面板：打开时整面板替换内容区（Esc 先关面板） -->
    <div
      v-if="study.visible"
      class="popup-scroll"
    >
      <StudyPanel
        :analysis="study.data"
        :loading="study.loading"
        :error="study.error"
        @close="closeStudy"
      />
    </div>

    <!-- 加载中：原文先行，流式译文增量填充 -->
    <div
      v-else-if="state.status === 'loading'"
      class="popup-scroll"
    >
      <template v-if="state.sourceText">
        <p class="popup-source">
          {{ state.sourceText }}
        </p>
        <div class="popup-source-meta">
          <span class="chip">翻译中…</span>
        </div>
      </template>
      <div class="result-card">
        <div class="result-card-header">
          <span class="result-engine">译文</span>
          <span
            v-if="!streamed.text"
            class="spinner"
            aria-hidden="true"
          />
        </div>
        <p
          v-if="streamed.text"
          class="popup-translation"
        >
          {{ streamed.text }}
        </p>
        <p
          v-else
          class="popup-translation streaming-hint"
        >
          正在翻译…
        </p>
      </div>
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
        <span
          v-if="ocrSourceLabel"
          class="chip"
          :title="ocrSource?.visionNote ?? ''"
        >{{ ocrSourceLabel }}</span>
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
        <!-- 译文行：词典命中时词条卡片即全部内容（译文行本就是首条
             释义的纯文本，与卡片重复）；AI 结果才单独显示译文。 -->
        <p
          v-if="state.dictionaries.length === 0"
          class="popup-translation"
        >
          {{ state.translatedText }}
        </p>

        <!-- 词典视图：多词典切换 + 词条释义/例句（共享面板，词卡同款） -->
        <div
          v-if="state.dictionaries.length > 0"
          class="popup-dictionary"
        >
          <DictionaryEntryPanel
            :entries="state.dictionaries"
            @lookup-word="jumpToEntry"
            @active-change="onActiveEntryChange"
          />
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
