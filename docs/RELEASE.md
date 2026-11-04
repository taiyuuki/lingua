# 发版流程（CI 自动发版 + 应用内更新）

推送到 `master` 触发 CI（typecheck / lint / 前端测试 / clippy / Rust 测试）；
推送 `v*` tag 触发 [Release 工作流](../.github/workflows/release.yml)：
构建 macOS（universal）与 Windows 安装包，发布 GitHub Release，并生成
`latest.json`（应用内更新器的清单，endpoint 指向
`https://github.com/taiyuuki/lingua/releases/latest/download/latest.json`，
公钥在 `src-tauri/tauri.conf.json plugins.updater.pubkey`）。

## 首次配置（已完成）

- 仓库可见性：**public**（更新器匿名下载 Release 资产的前提；若转回
  private，endpoint 需改带 `Authorization` 请求头，见 Tauri 文档）。
- 更新器签名密钥对：`pnpm tauri signer generate -w ~/.tauri/lingua-updater.key`。
  私钥在本机 `~/.tauri/lingua-updater.key`（空密码），公钥已写入 tauri.conf.json。
- GitHub secrets（repo → Settings → Secrets and variables → Actions）：
  - `TAURI_SIGNING_PRIVATE_KEY`：私钥文件内容；
  - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：**无需创建**（密钥为空密码；
    GitHub 禁止创建空 secret，工作流中该 env 会解析为空字符串，正好匹配
    空密码。切勿用占位符填充——会被当作真实密码导致签名失败）。
  - `MACOS_SIGNING_P12`：macOS 签名证书（.p12，base64，含私钥），须与
    `src-tauri/tauri.conf.json > bundle > macOS > signingIdentity` 指向的
    同一枚 Apple Development 证书（两处 SHA-1 哈希需同步更新）。导出：
    钥匙串访问 → 登录钥匙串 → 选中该证书（双击查看指纹 / 有效期确认）→
    确认展开含私钥 → 右键导出为 .p12 并设密码，然后
    `base64 -i 导出.p12 | pbcopy` 粘贴为 secret；
  - `MACOS_SIGNING_P12_PASSWORD`：导出 .p12 时设置的密码。
  - 未配置 `MACOS_SIGNING_P12` 时 CI 回退 ad-hoc 签名（工作流输出警告）；
    缺点是 TCC 授权（辅助功能等）随每次发版失效。

> ⚠️ 私钥丢失后将无法签发任何可被旧版本安装的更新包——务必备份。

本地 `pnpm tauri build` 同样会生成更新包签名，需带私钥环境变量
（实测当前 CLI 只认内容变量，不认 `_PATH` 形式）：

```bash
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/lingua-updater.key)" pnpm tauri build
```

## 发版步骤

```bash
node scripts/release.mjs <新版本号>   # 同步 package.json / tauri.conf.json / Cargo.toml
git add -A && git commit -m "chore: release v<版本号>"
git tag v<版本号>
git push origin master --tags
```

Action 完成后核对 Release 资产包含：dmg / app.tar.gz + sig（macOS）、
NSIS 安装器 + sig（Windows）、`latest.json`。

## 应用内更新

- 启动 3 秒后静默检查；有新版本时侧边栏「设置」出现红点。
- 设置页「关于与更新」卡片：手动检查、下载（进度条）、安装后自动重启。
- macOS CI 包用 Apple Development 证书签名（与本地构建同一枚）：辅助功能
  等 TCC 授权跨版本保留。该证书非 Developer ID 且包未公证，首次安装 dmg
  仍需右键 → 打开（或系统设置「仍要打开」）绕过 Gatekeeper；应用内更新
  路径不受影响，更新包完整性由 minisign 签名保证，与代码签名无关。

## 验证更新链路（端到端）

1. 安装 v0.1.0（或当前版本）的 dmg；
2. 发一版更高版本（如 v0.1.1）；
3. 旧应用内应能检测到新版本并完成下载、重启升级。
