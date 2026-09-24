import { describe, expect, it } from "vitest";
import { describeError } from "./errors";

describe("describeError", () => {
  it("maps AppError payload kinds to friendly labels", () => {
    const text = describeError({ kind: "database", message: "locked" });
    expect(text).toContain("本地数据库错误");
    expect(text).toContain("locked");
  });

  it("falls back for unknown kinds", () => {
    const text = describeError({ kind: "future_kind", message: "boom" });
    expect(text).toContain("未知错误");
    expect(text).toContain("boom");
  });

  it("passes plain strings through", () => {
    expect(describeError("直接来自 Tauri 的字符串错误")).toBe(
      "直接来自 Tauri 的字符串错误",
    );
  });

  it("unwraps native Error instances", () => {
    expect(describeError(new Error("native failure"))).toBe("native failure");
  });

  it("stringifies anything else", () => {
    expect(describeError(42)).toBe("42");
  });
});
