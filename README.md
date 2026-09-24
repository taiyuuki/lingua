# Lingua

跨平台 AI 翻译与语言学习助手（划词翻译 + OCR 翻译 + AI + 词典 + 生词本）。

技术栈：**Tauri 2 + Vue 3 + TypeScript + Rust workspace + SQLite**。

> 当前状态：**Phase 0 —— 项目初始化与架构骨架**。业务功能（划词、翻译、
> Provider 实现、OCR、词典、生词本）按阶段逐步落地，见
> [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) 的阶段规划。

## 目录结构

```
lingua/
├── crates/                  # Rust workspace（业务核心，全部平台无关）
│   ├── lingua-core/         # 领域模型：AppError / Language / SelectedText / Translation*
│   ├── lingua-platform/     # 平台抽象：SelectionProvider trait、OS 描述
│   ├── lingua-ai/           # AI 抽象：AIProvider trait、ProviderConfig、Chat/Vision 类型
│   └── lingua-database/     # SQLite：连接管理、migration runner、settings
├── src-tauri/               # Tauri 应用壳（薄层：commands / 窗口 / 状态注入）
├── src/                     # Vue 前端
│   ├── lib/api.ts           # 后端命令类型化封装（唯一 IPC 通道）
│   ├── lib/errors.ts        # AppError → 用户可读文案
│   ├── stores/              # Pinia（按域拆分）
│   └── popup/               # Popup 窗口入口（popup.html）
├── scripts/generate_icon.py # 图标母版生成脚本
└── docs/ARCHITECTURE.md     # 架构决策与阶段规划
```

## 环境要求

- Rust 1.85+（`rustup`）
- Node.js 20.19+ / 22.12+，包管理器用 **pnpm**
- macOS：Xcode Command Line Tools
- Windows：MSVC 工具链 + WebView2（Win11 自带）
- Linux：`webkit2gtk-4.1`、`libappindicator` 等开发包（见 Tauri 官方指引）

## 常用命令

```bash
pnpm install            # 安装前端依赖
pnpm tauri dev          # 开发模式运行桌面应用
pnpm tauri build        # 打包发布版
pnpm build              # 仅前端构建（vue-tsc + vite）
pnpm lint               # ESLint
pnpm test               # vitest 前端测试

cargo check --workspace --all-targets   # Rust 快速检查
cargo test --workspace                  # Rust 测试（26 个）
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

## 日志

日志走 `tracing`，输出到 stdout（`pnpm tauri dev` 的终端可见）。
级别用环境变量控制，例如：

```bash
RUST_LOG=lingua=debug pnpm tauri dev
```

约束：API Key、用户划词文本、截图内容不写日志。

## Phase 0 验收状态

| 检查项 | 命令 | 状态 |
|--------|------|------|
| 前端安装 | `pnpm install` | ✅ |
| 前端构建 | `pnpm build` | ✅（main + popup 双入口） |
| Rust 检查 | `cargo check --workspace --all-targets` | ✅ |
| Rust 测试 | `cargo test --workspace` | ✅ 26 passed |
| Lint | `cargo clippy -D warnings` / `pnpm lint` | ✅ |
| 应用链接 | `cargo build --workspace` | ✅ |
