<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import {
  cancelOcr,
  getOcrBackground,
  onOcrBackgroundUpdated,
  performOcrTranslation,
} from "../lib/api";

// 快照式选区：整屏截图由后端在 start_ocr_capture 时截好，
// 这里把截图铺满窗口作为背景，用户在「冻结的屏幕」上框选。
// 坐标为 CSS 像素；上报时连同窗口 CSS 视口一起传给后端，
// 后端按「截图像素 / 视口」自校准缩放比（HiDPI 下不用猜）。
interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

const background = ref<string | null>(null);
const dragStart = reactive({ x: 0, y: 0 });
const rect = reactive<Rect>({ x: 0, y: 0, width: 0, height: 0 });
const dragging = ref(false);
let unlisten: (() => void) | null = null;

// 窗口是复用的：每次 start_ocr_capture 后端会发通知，这里重新拉取，
// 保证背景始终是最新一次的截图。
async function refreshBackground(): Promise<void> {
  try {
    background.value = await getOcrBackground();
  } catch {
    background.value = null;
  }
}

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
  void performOcrTranslation(
    x,
    y,
    width,
    height,
    window.innerWidth,
    window.innerHeight,
  );
}

function onKeyDown(event: KeyboardEvent): void {
  if (event.key === "Escape") {
    void cancelOcr();
  }
}

onMounted(async () => {
  window.addEventListener("mousedown", onMouseDown);
  window.addEventListener("mousemove", onMouseMove);
  window.addEventListener("mouseup", onMouseUp);
  window.addEventListener("keydown", onKeyDown);
  const stop = await onOcrBackgroundUpdated(() => {
    // 先清旧图：宁可短暂黑屏，也不能在旧截图上框选。
    background.value = null;
    void refreshBackground();
  });
  unlisten = stop;
  await refreshBackground();
});

onUnmounted(() => {
  window.removeEventListener("mousedown", onMouseDown);
  window.removeEventListener("mousemove", onMouseMove);
  window.removeEventListener("mouseup", onMouseUp);
  window.removeEventListener("keydown", onKeyDown);
  unlisten?.();
});
</script>

<template>
  <div
    class="ocr-overlay"
    @contextmenu.prevent="cancelOcr()"
  >
    <img
      v-if="background"
      class="ocr-bg"
      :src="background"
      alt=""
      draggable="false"
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
