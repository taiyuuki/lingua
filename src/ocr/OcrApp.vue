<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import {
  cancelOcr,
  emitOcrSelection,
  getOcrSurface,
  onOcrBackgroundUpdated,
  onOcrSelection,
  performOcrTranslation,
  type OcrSelection,
  type OcrSurface,
} from "../lib/api";
import {
  selectionBetween,
  toCssRect,
  toVirtual,
  type OverlayFrame,
  type Rect,
} from "./selection";

// 快照式选区：各显示器的截图由后端在 start_ocr_capture 时截好，
// 每块屏一个窗口，把本屏那张截图铺满作为背景，用户在「冻结的屏幕」上框选。
//
// 多屏：选区统一用「虚拟桌面逻辑坐标」记录，拖拽时广播给其它 overlay ——
// 每个窗口只画落在自己视口里的那一段，于是可以从一块屏起笔、拖到另一块屏
// 松手。坐标换算的纯函数与说明见 ./selection.ts。

/** 误点判定：小于这个尺寸的选区忽略，保持 overlay 等待重新选择。 */
const MIN_SELECTION = 8;

const surface = ref<OcrSurface | null>(null);
const background = ref<string | null>(null);
/** 选区（虚拟桌面逻辑坐标，恒为左上角原点 + 正宽高）。 */
const selection = reactive<Rect>({ x: 0, y: 0, width: 0, height: 0 });
/** 有进行中的拖拽（可能发生在别的 overlay 上）。 */
const selecting = ref(false);
/** 此刻由本窗口跟踪拖拽（鼠标在本窗口按下，或从别的屏移进来被接管）。 */
const dragging = ref(false);
const anchor = reactive({ x: 0, y: 0 });
let unlisten: Array<() => void> = [];
let broadcastTimer: number | null = null;
/** 最近一次由本窗口处理的拖拽指针事件的时间；与远端广播的 `at` 比新旧。 */
let lastPointerAt = 0;

// 窗口是复用的：每次 start_ocr_capture 后端会发通知，这里重新拉取，
// 保证背景始终是最新一次的截图。
async function refreshBackground(): Promise<void> {
  try {
    surface.value = await getOcrSurface();
    background.value = surface.value?.background ?? null;
  } catch {
    surface.value = null;
    background.value = null;
  }
}

/** 本窗口的 overlay：铺满的显示器 + 当前 CSS 视口；没有活动会话时为 null。 */
function currentFrame(): OverlayFrame | null {
  const display = surface.value?.rect;
  return display
    ? {
        display,
        viewport: { width: window.innerWidth, height: window.innerHeight },
      }
    : null;
}

const hasSelection = computed(
  () => selecting.value && selection.width > 0 && selection.height > 0,
);

// 选区是全局坐标，换算回本窗口的 CSS 坐标再落位；跨屏时可以越出视口。
const rectStyle = computed(() => {
  const frame = currentFrame();
  const css = frame ? toCssRect(selection, frame) : { x: 0, y: 0, width: 0, height: 0 };
  return {
    left: `${css.x}px`,
    top: `${css.y}px`,
    width: `${css.width}px`,
    height: `${css.height}px`,
  };
});

function cancelBroadcast(): void {
  if (broadcastTimer !== null) {
    window.clearTimeout(broadcastTimer);
    broadcastTimer = null;
  }
}

function clearSelection(): void {
  cancelBroadcast();
  lastPointerAt = 0;
  selecting.value = false;
  dragging.value = false;
  Object.assign(selection, { x: 0, y: 0, width: 0, height: 0 });
}

/**
 * 广播节流：16ms 内最多发一次，选区在本地立即生效、远端按节流同步。
 * 用定时器而不是 requestAnimationFrame —— 窗口被系统节流时动画帧可能
 * 一秒才跑一次，输入相关的同步不能押在它身上。
 */
function scheduleBroadcast(): void {
  if (broadcastTimer !== null) {
    return;
  }
  broadcastTimer = window.setTimeout(flushSelection, 16);
}

function flushSelection(): void {
  broadcastTimer = null;
  // 排队期间拖拽已结束：结束时的 null 广播才是最终状态。
  if (!dragging.value) {
    return;
  }
  void emitOcrSelection({
    source: surface.value?.monitorIndex ?? -1,
    at: lastPointerAt,
    anchorX: anchor.x,
    anchorY: anchor.y,
    ...selection,
  });
}

function cancelDrag(): void {
  clearSelection();
  void emitOcrSelection(null);
}

function onMouseDown(event: MouseEvent): void {
  const frame = currentFrame();
  if (event.button !== 0 || !frame) {
    return;
  }
  const point = toVirtual({ x: event.clientX, y: event.clientY }, frame);
  dragging.value = true;
  selecting.value = true;
  lastPointerAt = Date.now();
  anchor.x = point.x;
  anchor.y = point.y;
  Object.assign(selection, { x: point.x, y: point.y, width: 0, height: 0 });
  // 起笔立即广播、不经节流：别的屏必须在鼠标移过去之前就知道拖拽已开始、
  // 锚点在哪（接管拖拽要用），这一下不能等定时器。
  cancelBroadcast();
  flushSelection();
}

function onMouseMove(event: MouseEvent): void {
  if (!dragging.value) {
    // 鼠标按着从别的屏移进来（起笔的窗口没把事件一路跟到底时）：接管拖拽。
    if (selecting.value && event.buttons === 1) {
      dragging.value = true;
    } else {
      return;
    }
  }
  if (event.buttons === 0) {
    // 在屏幕间隙里松开了鼠标，mouseup 没有窗口收到：取消这次拖拽。
    cancelDrag();
    return;
  }
  const frame = currentFrame();
  if (!frame) {
    return;
  }
  const point = toVirtual({ x: event.clientX, y: event.clientY }, frame);
  lastPointerAt = Date.now();
  Object.assign(selection, selectionBetween(anchor, point));
  scheduleBroadcast();
}

function onMouseUp(): void {
  if (!dragging.value) {
    return;
  }
  const frame = currentFrame();
  if (
    !frame ||
    selection.width < MIN_SELECTION ||
    selection.height < MIN_SELECTION
  ) {
    cancelDrag(); // 误点：清掉选区，保持 overlay 等待重新选择
    return;
  }
  const local = toCssRect(selection, frame);
  clearSelection();
  // 先让其它 overlay 复位，再以本窗口的 CSS 坐标提交（跨屏时可以越出本屏，
  // 后端按本屏原点换算回虚拟桌面坐标）。
  void emitOcrSelection(null);
  void performOcrTranslation(
    local.x,
    local.y,
    local.width,
    local.height,
    frame.viewport.width,
    frame.viewport.height,
  );
}

/**
 * 别的 overlay 上的拖拽：只负责把落在自己视口里的那一段画出来。
 * 只忽略自己发出的回声 —— 起笔的窗口在拖拽被别的屏接管后同样要跟着走，
 * 否则它屏上那半截选区会停在旧位置。
 */
function applyRemote(remote: OcrSelection | null): void {
  if (remote === null) {
    // 拖拽已结束（接管方松手 / 取消）：本窗口即使还在等自己的 mouseup
    // （鼠标已经移去了别的屏），这也是同一次拖拽，一并复位。
    clearSelection();
    return;
  }
  if (remote.source === surface.value?.monitorIndex) {
    return;
  }
  if (dragging.value && remote.at < lastPointerAt) {
    // 对方的指针信息比本窗口的旧：它的节流广播还在路上时，鼠标已经移到了
    // 本屏。以最新的指针为准。
    return;
  }
  anchor.x = remote.anchorX;
  anchor.y = remote.anchorY;
  // 本窗口的状态现在反映的是对方那次指针事件。
  lastPointerAt = Math.max(lastPointerAt, remote.at);
  Object.assign(selection, {
    x: remote.x,
    y: remote.y,
    width: remote.width,
    height: remote.height,
  });
  selecting.value = true;
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
  unlisten = [
    await onOcrBackgroundUpdated(() => {
      // 新一轮截图：清掉上一轮的选区；先清旧图——宁可短暂黑屏，
      // 也不能在旧截图上框选。
      clearSelection();
      background.value = null;
      void refreshBackground();
    }),
    await onOcrSelection(applyRemote),
  ];
  await refreshBackground();
});

onUnmounted(() => {
  window.removeEventListener("mousedown", onMouseDown);
  window.removeEventListener("mousemove", onMouseMove);
  window.removeEventListener("mouseup", onMouseUp);
  window.removeEventListener("keydown", onKeyDown);
  for (const stop of unlisten) {
    stop();
  }
  unlisten = [];
  cancelBroadcast();
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
      v-show="hasSelection"
      class="ocr-rect"
      :style="rectStyle"
    />
    <div
      v-show="!selecting"
      class="ocr-hint"
    >
      拖拽框选要翻译的区域 · 可跨屏拖拽 · Esc 或右键取消
    </div>
  </div>
</template>
