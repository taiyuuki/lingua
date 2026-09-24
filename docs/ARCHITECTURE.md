# Lingua 架构文档

## 1. 分层

```
Frontend (Vue 3, <script setup>, Pinia)
    ↓  invoke / events（唯一通道：src/lib/api.ts）
Tauri Commands / Events        （src-tauri，薄接线层）
    ↓
Application Services           （Phase 1 起在 crates 中实现）
    ↓
Domain Services / Engine       （Translation Engine、AI Router、SRS…）
    ↓
Providers / Infrastructure     （SelectionProvider / AIProvider / OcrProvider / Database）
    ↓
OS / Network / SQLite / Models
```

硬性规则（已由项目结构物理保证）：

1. UI 不直接调用第三方 AI API —— 前端没有任何 HTTP 客户端依赖。
2. UI 不直接操作数据库 —— 前端没有任何 SQL/SQLite 依赖。
3. UI 不依赖具体 OCR / 翻译 / AI 实现 —— 一切经 Rust trait 抽象。
4. 平台相关代码只存在于 `lingua-platform`（未来含 ocr 的平台实现），
   用 `cfg(target_os)` 隔离，不散落在业务代码。

## 2. Crate 划分与阶段映射

| Crate | 职责 | 状态 |
|-------|------|------|
| `lingua-core` | 领域模型：`AppError`、`Language`、`SelectedText`、`TranslationRequest/Result`，IPC 序列化契约（camelCase） | ✅ Phase 0 |
| `lingua-platform` | `SelectionProvider` trait、OS 描述；Phase 1 落地 macOS AX / Windows UIA / Linux AT-SPI + 剪贴板 fallback | ✅ 边界已定 |
| `lingua-ai` | `AIProvider` trait、`ProviderConfig`（api_key 不序列化）、Chat/Vision 类型；Phase 1 落地 OpenAI-compatible 实现 | ✅ 边界已定 |
| `lingua-database` | SQLite（rusqlite bundled）、migration runner、settings KV | ✅ Phase 0 |
| `lingua` (src-tauri) | Tauri 壳：commands、tracing、窗口管理 | ✅ Phase 0 |

后续按阶段新增（不为形式提前建空 crate）：

- **Phase 1** `lingua-translation`（Translation Engine + Prompt 模板）；
  全局快捷键引入 `tauri-plugin-global-shortcut`。
- **Phase 2** `lingua-secrets`（keyring：Keychain / Credential Manager / Secret Service），
  `providers` 表 + 多 Provider 管理 UI。
- **Phase 5** `lingua-ocr`（`OcrProvider` trait + PaddleOCR）与 `lingua-screenshot`
  （截屏选区，平台差异大：macOS ScreenCaptureKit/CGWindowList、Windows GDI/DXGI、
  Linux X11/Wayland 分治）。
- **Phase 3/4/8/9** dictionary、vocabulary、history、SRS 视规模决定
  独立 crate 或并入相邻 crate，边界先行（先有模块边界，再有 crate 拆分）。

### 为什么不现在建全部 crate

需求约束「不要为了形式主义拆分 crate」。空的 screenshot/ocr/dictionary
crate 没有任何可约束的实现细节，过早建立只会产生错误的占位接口；
等真实需求出现时建 crate，trait 按真实调用方需求设计。

## 3. 关键技术决策（ADR 摘要）

| 决策 | 理由 |
|------|------|
| **rusqlite（bundled）而非 sqlx** | bundled SQLite 静态编译进二进制，三个平台零系统依赖，CI 稳定；迁移用 `PRAGMA user_version` + 嵌入式 SQL 文件（每个 migration 独立事务、不可变、防降级），行为完全可测试。异步场景经 `spawn_blocking` 包裹，桌面应用单连接 + WAL 足够。 |
| **async-trait 而非原生 AFIT** | Provider 注册表需要 `Box<dyn AIProvider>` 动态分派，原生 async fn in trait 目前不是 object-safe。 |
| **API Key 永不落库** | `ProviderConfig.api_key` 标 `#[serde(skip)]`，测试强制断言序列化输出不含密钥；Phase 2 引入 OS 密钥库（Keychain / Credential Manager / Secret Service）。 |
| **错误即契约** | `AppError`（thiserror）`#[non_exhaustive]`，序列化为 `{kind, message}`；前端 `describeError` 按 kind 映射文案。workspace lints 对 `unwrap/expect/panic` 告警（测试模块显式豁免）。 |
| **IPC 统一 camelCase** | 所有跨边界类型 `#[serde(rename_all = "camelCase")]`，与 TS 接口一致。 |
| **不用 UI 组件库** | 需求要求轻量桌面风格；Popup 对体积与启动速度敏感。当前用设计令牌 + 手写样式；若后续 Settings/词汇管理页表单复杂度上升，再评估引入（会先说明理由）。 |
| **edition 2024 / clippy 收紧** | rustc 1.97，workspace 统一 lints（unsafe deny，unwrap/expect/panic warn）。 |
| **CSP 暂为 null** | Phase 0 无外部资源加载；Phase 1 接入远程 Provider 前收紧并补充。 |

## 4. 窗口系统

| 窗口 | label | 入口 | 状态 |
|------|-------|------|------|
| 主窗口 | `main` | `tauri.conf.json` | ✅ Phase 0 |
| Popup | `popup` | `popup.html`（Vite 多入口） | ✅ 骨架：无边框、置顶、skip taskbar、失焦自动隐藏（`on_window_event`）；Phase 1 加按选区 `rect` 定位与 Esc 关闭 |
| OCR 选区 | `ocr-selection` | Phase 5 | 计划：全屏透明 overlay，独立实现注意 Wayland 限制 |
| Settings / Vocabulary | 复用 main 路由或独立窗口 | Phase 2/4 | 待定 |

Capabilities（`src-tauri/capabilities/default.json`）当前只放行
`main` / `popup` + `core:default`；新增窗口或插件权限时同步维护。

## 5. 平台差异备忘（Phase 1/5/10 的输入）

- **全局快捷键**：用 `tauri-plugin-global-shortcut`（官方插件），不要自写平台钩子。
- **划词**：macOS 需要用户在系统设置授予 **辅助功能（Accessibility）** 权限；
  截屏需要 **屏幕录制** 权限。权限检测/引导应走 `AppError::Permission`。
  剪贴板 fallback 必须备份并恢复剪贴板。
- **Wayland**：无可靠的全局选区坐标与全局快捷键方案，Popup 定位能力受限；
  Phase 10 按桌面环境（GNOME/KDE）逐个处理，X11 下能力完整。
- **托盘/自启**：Phase 10（`tauri-plugin-autostart`、托盘能力）。

## 6. Phase 1 待办（下一步）

1. `lingua-platform`：macOS `SelectionProvider`（AX + 剪贴板 fallback）。
2. `lingua-ai`：`OpenAiCompatibleProvider`（reqwest + serde，非流式）。
3. `lingua-translation`：Translation Engine 骨架 + prompts/ 目录（Prompt 与代码分离）。
4. `tauri-plugin-global-shortcut` 接入，默认快捷键（如 `Alt+T`）可配置。
5. Popup：接收选区事件 → 调翻译 → 展示；按 `rect` 定位；Esc 隐藏。
6. 最小 Settings：Provider 配置（base_url / api_key / model），api_key 进 Keychain。
