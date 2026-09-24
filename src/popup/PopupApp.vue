<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { hidePopup, latestPopupState, onPopupState, type PopupState } from "../lib/api";

// Popup 是独立的轻量展示窗口：本地状态 + 事件/快照双通道同步。
// Rust 侧保证：先写快照再广播事件，因此这里先订阅再拉快照不会丢消息。
const state = ref<PopupState>({ status: "loading" });
const copied = ref(false);
let unlisten: (() => void) | null = null;

const detectedLanguageLabel = computed(() => {
  if (state.value.status !== "result" || !state.value.detectedLanguage) {
    return null;
  }
  return state.value.detectedLanguage;
});

async function copyTranslation(): Promise<void> {
  if (state.value.status !== "result") {
    return;
  }
  try {
    await navigator.clipboard.writeText(state.value.translatedText);
    copied.value = true;
    window.setTimeout(() => {
      copied.value = false;
    }, 1500);
  } catch {
    // 剪贴板不可用时静默忽略（Phase 1 可接受）。
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
    <!-- 加载中 -->
    <template v-if="state.status === 'loading'">
      <div class="popup-loading">
        <span
          class="spinner"
          aria-hidden="true"
        />
        <span>正在翻译…</span>
      </div>
    </template>

    <!-- 翻译结果：内容整体滚动，操作按钮固定在底部 -->
    <template v-else-if="state.status === 'result'">
      <div class="popup-scroll">
        <div class="popup-section">
          <p class="popup-source">
            {{ state.sourceText }}
          </p>
          <div class="popup-meta">
            <span
              v-if="detectedLanguageLabel"
              class="chip"
            >{{ detectedLanguageLabel }}</span>
          </div>
        </div>

        <p class="popup-translation">
          {{ state.translatedText }}
        </p>

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

        <p
          v-if="state.explanation"
          class="popup-explanation"
        >
          {{ state.explanation }}
        </p>
      </div>

      <div class="popup-actions">
        <button
          class="btn btn-small"
          type="button"
          @click="copyTranslation"
        >
          {{ copied ? "已复制" : "复制译文" }}
        </button>
        <button
          class="btn btn-small btn-secondary"
          type="button"
          @click="hidePopup()"
        >
          关闭
        </button>
      </div>
    </template>

    <!-- 错误 -->
    <template v-else>
      <div class="popup-error">
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
          macOS 需要在「系统设置 → 隐私与安全性 → 辅助功能」中授权本应用。
        </p>
      </div>
    </template>
  </div>
</template>
