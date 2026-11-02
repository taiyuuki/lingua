#!/usr/bin/env node
// 发版辅助：同步三处版本号并给出打 tag 的后续命令。
//
// 用法：node scripts/release.mjs <新版本号>   例如 0.2.0
//
// 机械操作 + 断言（AGENTS.md 规则 3）：
//   - package.json / src-tauri/tauri.conf.json 走 JSON 精确读写；
//   - Cargo.toml 走精确行匹配，替换计数不为 1 即报错退出；
//   - 版本号必须符合 x.y.z 语义化格式。
// 脚本不做 git 操作（commit / tag / push 由人工确认后执行）。

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const version = process.argv[2];

if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  console.error("用法: node scripts/release.mjs <x.y.z>  （如 0.2.0）");
  process.exit(1);
}

// --- package.json ---
const packagePath = path.join(repoRoot, "package.json");
const pkg = JSON.parse(readFileSync(packagePath, "utf8"));
pkg.version = version;
writeFileSync(packagePath, JSON.stringify(pkg, null, 2) + "\n");

// --- src-tauri/tauri.conf.json ---
const confPath = path.join(repoRoot, "src-tauri/tauri.conf.json");
const conf = JSON.parse(readFileSync(confPath, "utf8"));
conf.version = version;
writeFileSync(confPath, JSON.stringify(conf, null, 2) + "\n");

// --- Cargo.toml（workspace.package 的 version，TOML 保持注释与顺序只能行级替换） ---
const cargoPath = path.join(repoRoot, "Cargo.toml");
const cargo = readFileSync(cargoPath, "utf8");
const versionLine = /^version = "\d+\.\d+\.\d+"$/m;
// 注意：RegExp 构造器传 RegExp 对象时不继承原 flags，必须显式给 "gm"。
const matches = cargo.match(new RegExp(versionLine.source, "gm")) ?? [];
if (matches.length !== 1) {
  // 回滚前两个 JSON 文件，避免半更新状态。
  console.error(`Cargo.toml 中 version 行匹配到 ${matches.length} 处（预期 1），已中止。`);
  process.exit(1);
}
const patched = cargo.replace(versionLine, `version = "${version}"`);
if (patched === cargo && matches[0] !== `version = "${version}"`) {
  console.error("Cargo.toml 替换未生效，已中止。");
  process.exit(1);
}
writeFileSync(cargoPath, patched);

// --- 回读断言 ---
for (const [label, file, extract] of [
  ["package.json", packagePath, (c) => JSON.parse(c).version],
  ["tauri.conf.json", confPath, (c) => JSON.parse(c).version],
  ["Cargo.toml", cargoPath, (c) => c.match(/^version = "(.+)"$/m)?.[1]],
]) {
  const got = extract(readFileSync(file, "utf8"));
  if (got !== version) {
    console.error(`${label} 回读版本为 ${got}（预期 ${version}），请人工检查。`);
    process.exit(1);
  }
}

console.log(`三处版本号已同步为 ${version}：`);
console.log("  package.json / src-tauri/tauri.conf.json / Cargo.toml");
console.log("\n后续步骤：");
console.log("  git add -A && git commit -m \"chore: release v" + version + "\"");
console.log("  git tag v" + version);
console.log("  git push origin master --tags");
console.log("\n推送 tag 后 GitHub Actions 会构建并发布 Release（含 latest.json）。");
