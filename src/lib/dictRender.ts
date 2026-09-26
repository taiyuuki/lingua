/**
 * 词典词条富文本渲染的共享工具 —— Popup 词典区与生词本词卡共用。
 *
 * 内容在导入期经 ammonia 白名单净化，渲染端可信；这里只做平台
 * 适配（Windows WebView2 的自定义 scheme 子资源形态）与交互
 * 分发（发音播放 / entry:// 链接语义解析）。
 */

import type { DictionaryEntryDto } from "./api";
import { lookupWordDictionaries } from "./api";

// Windows 的 WebView2 不支持自定义 scheme 的子资源加载（wry 仅在
// 导航时做 scheme 转换），渲染前把资源 URL 转成 http://mdx-res.localhost
// 形态（与 tauri asset 协议的 convertFileSrc 同一约定）。
// 库里存平台无关的 mdx-res:// 形态，换平台无需重导。
export const MDX_RES_ORIGIN = navigator.userAgent.includes("Windows")
  ? "http://mdx-res.localhost/"
  : "mdx-res://localhost/";

export function renderRichHtml(html: string): string {
  return html.replaceAll("mdx-res://localhost/", MDX_RES_ORIGIN);
}

/** 单个 mdx-res 资源 URL → 当前平台可加载形态（喇叭按钮直连播放用）。 */
export function platformResourceUrl(url: string): string {
  return url.replace("mdx-res://localhost/", MDX_RES_ORIGIN);
}

/** 音频资源链接判定（sound:// 在导入期已改写为 mdx-res 资源 URL）。 */
const AUDIO_URL = /\.(mp3|wav|ogg|oga|m4a|aac|spx|opus|flac|webm)(\?|$)/i;

/** 同时只播一个：点新喇叭先停上一个。 */
let currentAudio: HTMLAudioElement | null = null;

export function playResourceAudio(url: string): void {
  currentAudio?.pause();
  currentAudio = new Audio(url);
  void currentAudio.play().catch(() => {
    // 音频缺失/解码失败静默
  });
}

/** entry:// 链接语义：词条内锚点 / 跳词。 */
export type EntryLink =
  | { kind: "anchor"; anchor: string }
  | { kind: "word"; word: string }
  | { kind: "audio"; url: string }
  | null;

export function parseEntryLink(href: string): EntryLink {
  if (href.startsWith("entry://#")) {
    return { kind: "anchor", anchor: href.slice("entry://#".length) };
  }
  if (href.startsWith("entry://")) {
    return { kind: "word", word: decodeURIComponent(href.replace(/^entry:\/\//, "")).trim() };
  }
  if (AUDIO_URL.test(href) && href.includes("mdx-res")) {
    return { kind: "audio", url: href };
  }
  return null;
}

/** 富文本区的点击分发。返回解析出的链接语义（调用方据此跳词/滚动）。 */
export function dispatchRichClick(event: MouseEvent): EntryLink {
  const anchor = (event.target as HTMLElement | null)?.closest?.(
    "a[href]",
  ) as HTMLAnchorElement | null;
  if (!anchor) {
    return null;
  }
  event.preventDefault();
  const href = anchor.getAttribute("href") ?? "";
  const link = parseEntryLink(href);
  if (link?.kind === "audio") {
    // anchor.href 是解析后的绝对地址（已含平台形态）
    playResourceAudio(anchor.href);
    return null;
  }
  return link;
}

/** 词条内锚点滚动定位（entry://#xxx → [name]/[id]）。 */
export function scrollToAnchor(container: HTMLElement | null, anchor: string): void {
  if (!container) {
    return;
  }
  const anchorId = CSS.escape(anchor);
  const target = container.querySelector(`[name="${anchorId}"], [id="${anchorId}"]`);
  target?.scrollIntoView({ block: "start", behavior: "smooth" });
}

/** 跳词/词卡：查全部命中词典（每本一条，导入顺序）。未命中返回空数组。
 *  带语言过滤查不到时回退全词典——生词记录的语言可能是 und
 *  （早期 AI 路径没检测出语言）或与词典导入语言不一致。 */
export async function lookupEntriesForWord(
  word: string,
  language?: string | null,
): Promise<DictionaryEntryDto[]> {
  if (!word) {
    return [];
  }
  try {
    const entries = await lookupWordDictionaries(word, language);
    if (entries.length > 0 || !language) {
      return entries;
    }
    return await lookupWordDictionaries(word, null);
  } catch {
    return [];
  }
}

/** 释义是否需要折叠（纯文本路径；富文本不折叠）。 */
export function isLongDefinition(text: string): boolean {
  return text.length > 160 || text.split("\n").length > 5;
}
