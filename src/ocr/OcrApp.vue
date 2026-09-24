<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { cancelOcr, performOcrTranslation } from "../lib/api";

// 全屏透明选区 overlay：拖拽框选 → 调 perform_ocr_translation；
// Esc 取消。坐标为 CSS 像素 = 全局逻辑点（与 screencapture -R 一致）。
interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

const dragStart = reactive({ x: 0, y: 0 });
const rect = reactive<Rect>({ x: 0, y: 0, width: 0, height: 0 });
const dragging = ref(false);

const rectStyle = computed(() => ({
  left: `${Math.min(rect.x, rect.x + rect.width)}px`,
  top: `${Math.min(rect.y, rect.y + rect.height)}px`,
  width: `${Math.abs(rect.width)}px`,
  height: `${Math.abs(rect.height)}px`,
}));

function onMouseDown(event: MouseEvent): void {
  if (event.button !== 0) {
    return;
  }
  dragging.value = true;
  dragStart.x = event.clientX;
  dragStart.y = event.clientY;
  rect.x = event.clientX;
  rect.y = event.clientY;
  rect.width = 0;
  rect.height = 0;
}

function onMouseMove(event: MouseEvent): void {
  if (!dragging.value) {
    return;
  }
  rect.width = event.clientX - dragStart.x;
  rect.height = event.clientY - dragStart.y;
}

function onMouseUp(): void {
  if (!dragging.value) {
    return;
  }
  dragging.value = false;
  const x = Math.min(rect.x, rect.x + rect.width);
  const y = Math.min(rect.y, rect.y + rect.height);
  const width = Math.abs(rect.width);
  const height = Math.abs(rect.height);
  if (width < 8 || height < 8) {
    return; // 误点：保持 overlay 等待重新选择
  }
  // 后端会先隐藏本窗口再截图。
  void performOcrTranslation(x, y, width, height);
}

function onKeyDown(event: KeyboardEvent): void {
  if (event.key === "Escape") {
    void cancelOcr();
  }
}

onMounted(() => {
  window.addEventListener("mousedown", onMouseDown);
  window.addEventListener("mousemove", onMouseMove);
  window.addEventListener("mouseup", onMouseUp);
  window.addEventListener("keydown", onKeyDown);
});

onUnmounted(() => {
  window.removeEventListener("mousedown", onMouseDown);
  window.removeEventListener("mousemove", onMouseMove);
  window.removeEventListener("mouseup", onMouseUp);
  window.removeEventListener("keydown", onKeyDown);
});
</script>

<template>
  <div
    class="ocr-overlay"
    @contextmenu.prevent="cancelOcr()"
  >
    <div
      v-show="dragging"
      class="ocr-rect"
      :style="rectStyle"
    />
    <div class="ocr-hint">
      拖拽框选要翻译的区域 · Esc 或右键取消
    </div>
  </div>
</template>
