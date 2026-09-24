// 从 assets/icon.svg 渲染 1024×1024 PNG（`tauri icon` 的输入）。
// 用法：pnpm icon（见 package.json，渲染后自动生成 src-tauri/icons/ 全格式）。
// 「文」依赖系统字体（微软雅黑/黑体）；CI 等无中文字体的环境
// 渲染结果不对，本地跑即可。
import { Resvg } from "@resvg/resvg-js";
import { readFile, writeFile } from "node:fs/promises";

const svg = await readFile("assets/icon.svg", "utf8");
const resvg = new Resvg(svg, {
  fitTo: { mode: "width", value: 1024 },
  font: { loadSystemFonts: true },
});
const png = resvg.render().asPng();
await writeFile("assets/icon-1024.png", png);
console.log(`assets/icon-1024.png rendered (${png.length} bytes)`);
