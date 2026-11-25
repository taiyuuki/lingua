/**
 * OCR 选区的坐标换算（纯函数，无 Tauri / DOM 依赖）。
 *
 * 两套坐标系：
 * - **overlay CSS 像素**：每个 overlay 窗口自己的视口坐标，鼠标事件给的就是它；
 * - **虚拟桌面逻辑坐标**：所有 overlay 共用的全局坐标（主屏左上角为原点，
 *   排在左侧 / 上方的屏为负）。选区用它记录，并在窗口间广播。
 */

export interface Point {
  x: number;
  y: number;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** 某块屏的 overlay：它铺满的显示器（虚拟桌面逻辑坐标）+ 它的 CSS 视口尺寸。 */
export interface OverlayFrame {
  display: Rect;
  viewport: { width: number; height: number };
}

/** 每个逻辑单位对应多少 CSS 像素；视口尺寸非法时按 1:1（与后端换算一致）。 */
function cssPerLogical(length: number, viewport: number): number {
  return Number.isFinite(viewport) && viewport >= 1 ? length / viewport : 1;
}

/** overlay 的 CSS 坐标 → 虚拟桌面逻辑坐标（越出该窗口时同样成立）。 */
export function toVirtual(point: Point, frame: OverlayFrame): Point {
  return {
    x: frame.display.x + point.x * cssPerLogical(frame.display.width, frame.viewport.width),
    y: frame.display.y + point.y * cssPerLogical(frame.display.height, frame.viewport.height),
  };
}

/**
 * 虚拟桌面逻辑矩形 → 该 overlay 的 CSS 像素矩形。
 * 落在视口外的部分保留负偏移 / 溢出尺寸，交给浏览器裁剪。
 */
export function toCssRect(rect: Rect, frame: OverlayFrame): Rect {
  return {
    x: (rect.x - frame.display.x) / cssPerLogical(frame.display.width, frame.viewport.width),
    y: (rect.y - frame.display.y) / cssPerLogical(frame.display.height, frame.viewport.height),
    width: rect.width / cssPerLogical(frame.display.width, frame.viewport.width),
    height: rect.height / cssPerLogical(frame.display.height, frame.viewport.height),
  };
}

/** 锚点到当前点围成的选区：恒为左上角原点 + 正宽高（任意方向拖拽）。 */
export function selectionBetween(anchor: Point, point: Point): Rect {
  return {
    x: Math.min(anchor.x, point.x),
    y: Math.min(anchor.y, point.y),
    width: Math.abs(point.x - anchor.x),
    height: Math.abs(point.y - anchor.y),
  };
}
