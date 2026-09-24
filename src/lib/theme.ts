export type Theme = "light" | "dark";

/** 把主题应用到当前窗口（写 <html data-theme>，CSS 令牌随之切换）。 */
export function applyTheme(theme: string): void {
  document.documentElement.dataset.theme = theme === "light" ? "light" : "dark";
}

export function normalizeTheme(value: string | null | undefined): Theme {
  return value === "light" ? "light" : "dark";
}
