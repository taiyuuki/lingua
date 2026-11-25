import { describe, expect, it } from "vitest";
import {
  selectionBetween,
  toCssRect,
  toVirtual,
  type OverlayFrame,
  type Point,
  type Rect,
} from "./selection";

/** 左右并排的两块 1× 屏：左屏在原点，右屏在 x = 1920。 */
const LEFT: OverlayFrame = {
  display: { x: 0, y: 0, width: 1920, height: 1080 },
  viewport: { width: 1920, height: 1080 },
};
const RIGHT: OverlayFrame = {
  display: { x: 1920, y: 0, width: 1920, height: 1080 },
  viewport: { width: 1920, height: 1080 },
};

/** 在某个 overlay 上从 `anchor` 拖到 `point`（CSS 像素），返回虚拟坐标选区。 */
function dragOn(frame: OverlayFrame, anchor: Point, point: Point): Rect {
  return selectionBetween(toVirtual(anchor, frame), toVirtual(point, frame));
}

describe("toVirtual", () => {
  it("offsets CSS pixels by the display origin", () => {
    expect(toVirtual({ x: 100, y: 50 }, RIGHT)).toEqual({ x: 2020, y: 50 });
  });

  it("supports a display left of the primary (negative coordinates)", () => {
    const leftOfPrimary: OverlayFrame = {
      display: { x: -1920, y: 0, width: 1920, height: 1080 },
      viewport: { width: 1920, height: 1080 },
    };
    expect(toVirtual({ x: 1910, y: 0 }, leftOfPrimary)).toEqual({ x: -10, y: 0 });
  });

  it("scales when the viewport is smaller than the display", () => {
    // Retina 类比例：1440×900 逻辑点铺在 720×450 的 CSS 视口里，1 逻辑点 = 0.5 CSS 像素。
    const scaled: OverlayFrame = {
      display: { x: 0, y: 0, width: 1440, height: 900 },
      viewport: { width: 720, height: 450 },
    };
    expect(toVirtual({ x: 100, y: 50 }, scaled)).toEqual({ x: 200, y: 100 });
  });

  it("falls back to 1:1 for an invalid viewport", () => {
    for (const viewport of [0, Number.NaN, -1]) {
      const frame: OverlayFrame = {
        display: { x: 10, y: 20, width: 1920, height: 1080 },
        viewport: { width: viewport, height: viewport },
      };
      expect(toVirtual({ x: 100, y: 50 }, frame)).toEqual({ x: 110, y: 70 });
    }
  });
});

describe("toCssRect", () => {
  it("subtracts the display origin", () => {
    const rect = { x: 2020, y: 50, width: 300, height: 100 };
    expect(toCssRect(rect, RIGHT)).toEqual({ x: 100, y: 50, width: 300, height: 100 });
  });

  it("keeps a negative offset for the part outside the viewport", () => {
    // 跨屏选区在右屏 overlay 上从屏幕左缘往左 190px 处开始。
    const rect = { x: 1730, y: 100, width: 200, height: 200 };
    expect(toCssRect(rect, RIGHT)).toEqual({ x: -190, y: 100, width: 200, height: 200 });
  });

  it("inverts toVirtual", () => {
    const scaled: OverlayFrame = {
      display: { x: 0, y: 0, width: 1440, height: 900 },
      viewport: { width: 720, height: 450 },
    };
    expect(toCssRect({ x: 200, y: 100, width: 400, height: 200 }, scaled)).toEqual({
      x: 100,
      y: 50,
      width: 200,
      height: 100,
    });
  });
});

describe("selectionBetween", () => {
  it("normalizes every drag direction to origin + positive size", () => {
    const anchor = { x: 100, y: 100 };
    expect(selectionBetween(anchor, { x: 150, y: 40 })).toEqual({
      x: 100,
      y: 40,
      width: 50,
      height: 60,
    });
    expect(selectionBetween(anchor, { x: 30, y: 180 })).toEqual({
      x: 30,
      y: 100,
      width: 70,
      height: 80,
    });
  });

  it("yields an empty rect for a click without movement", () => {
    expect(selectionBetween({ x: 5, y: 5 }, { x: 5, y: 5 })).toEqual({
      x: 5,
      y: 5,
      width: 0,
      height: 0,
    });
  });
});

describe("cross-display drag", () => {
  it("maps a drag that starts on the right display and ends on the left one", () => {
    // 在右屏 overlay 上：从 (10, 100) 起笔，向左拖出窗口到 (-190, 300)。
    const selection = dragOn(RIGHT, { x: 10, y: 100 }, { x: -190, y: 300 });
    expect(selection).toEqual({ x: 1730, y: 100, width: 200, height: 200 });

    // 两块屏各自画落在自己视口里的那一段：左屏的右缘 + 右屏的左缘。
    expect(toCssRect(selection, LEFT)).toEqual({ x: 1730, y: 100, width: 200, height: 200 });
    expect(toCssRect(selection, RIGHT)).toEqual({ x: -190, y: 100, width: 200, height: 200 });
  });

  it("gives the same virtual selection regardless of which overlay tracks the drag", () => {
    // 起笔在右屏 (10, 100)，鼠标移到左屏的 (1730, 300) 上松手：左屏接管后算出的
    // 选区必须与右屏自己算的一致（同一个物理点 = 虚拟坐标 1730）。
    const fromRight = dragOn(RIGHT, { x: 10, y: 100 }, { x: -190, y: 300 });
    const fromLeft = selectionBetween(
      { x: 1930, y: 100 }, // 右屏广播的锚点（虚拟坐标）
      toVirtual({ x: 1730, y: 300 }, LEFT),
    );
    expect(fromLeft).toEqual(fromRight);
  });

  it("round-trips a single-display selection back to the reporting overlay", () => {
    const selection = dragOn(RIGHT, { x: 100, y: 100 }, { x: 400, y: 300 });
    expect(toCssRect(selection, RIGHT)).toEqual({ x: 100, y: 100, width: 300, height: 200 });
  });
});
