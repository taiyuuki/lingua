# Lingua

跨平台 AI 翻译与语言学习助手（划词翻译 + OCR 翻译 + AI + 词典 + 生词本）。

技术栈：**Tauri 2 + Vue 3 + TypeScript + Rust workspace + SQLite**。

> 当前状态：**Phase 5 —— 截图翻译（OCR）已完成**：框选屏幕区域 → OCR →
> AI 翻译 → Popup。另有划词翻译（词典优先）、多 Provider 路由、生词本。
> 在任意应用选中文字 → 全局快捷键 → Popup 显示 AI 翻译。
> 架构决策与阶段规划见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

## 目录结构

```
lingua/
├── crates/                  # Rust workspace（业务核心，全部平台无关）
│   ├── lingua-core/         # 领域模型：AppError / Language / SelectedText / Translation*
│   ├── lingua-platform/     # 平台抽象：SelectionProvider、macOS AX 与 Windows UIA/剪贴板实现、GDI 截图
│   ├── lingua-ai/           # AI 抽象：AIProvider trait、OpenAI-compatible 实现、AIRouter 任务路由
│   ├── lingua-translation/  # Translation Engine：词典优先路由 + prompts/ 模板
│   ├── lingua-dictionary/   # DictionaryProvider trait、SQLite 词典、TSV 导入器
│   ├── lingua-ocr/          # OcrProvider trait、Tesseract CLI / HTTP sidecar 实现
│   │                        # （生词本 DAO 在 lingua-database，SRS 数据字段已就位）
│   ├── lingua-database/     # SQLite：migration runner、settings、providers 表
│   └── lingua-secrets/      # OS 密钥库（Keychain / Credential Manager / Secret Service）
├── src-tauri/               # Tauri 应用壳：commands、全局快捷键、翻译 pipeline、Popup
├── src/                     # Vue 前端
│   ├── lib/api.ts           # 后端命令类型化封装（唯一 IPC 通道）
│   ├── lib/errors.ts        # AppError → 用户可读文案
│   ├── stores/              # Pinia（app / settings，按域拆分）
│   ├── popup/               # Popup 窗口入口（popup.html）
│   └── ocr/                 # OCR 选区 overlay 入口（ocr.html）
├── scripts/generate_icon.py # 图标母版生成脚本
└── docs/ARCHITECTURE.md     # 架构决策与阶段规划
```

## 使用方法

1. `pnpm tauri dev` 启动应用，在「AI Providers」里添加 Provider（baseUrl /
   模型 / API Key，Key 存入系统钥匙串），可用「测试」按钮做连接检查；
   在「路由与快捷键」里选默认 Provider / 翻译任务 Provider / 目标语言。
2. 在任意应用中选中文本，按 `alt+t`（可在设置里改）。
3. 翻译结果在光标所在屏幕居中的 Popup 中显示（多显示器跟随光标）；
   `Esc` 或点击外部关闭。
4. 按 `alt+s`（可在设置里改）或点主窗口「截图翻译」按钮 → 在屏幕上
   拖拽框选 → 识别的文字自动走 AI 翻译并在 Popup 显示。快捷键入口
   截**光标所在**显示器（多屏幕用这个），按钮入口截主窗口所在屏。
   OCR 引擎在「路由与快捷键」里配置（默认本机 tesseract；PaddleOCR
   走 `scripts/paddleocr_server.py` sidecar，部署见 docs/PADDLEOCR.md）。
   识别为空或置信度偏低时，若配置了开启「视觉」能力的 Provider 会
   自动用它重新识别兜底；Popup 工具栏也可手动「用 Vision 重新识别」。
5. （可选）在「词典」卡片导入 TSV 词典：翻译单词时优先命中本地词典，
   Popup 显示词性、释义、例句。格式说明见导入表单内的提示。
6. macOS 首次使用需在「系统设置 → 隐私与安全性 → 辅助功能」授权本应用
   （Windows 划词与截图无需任何系统授权）。

## 环境要求

- Rust 1.85+（`rustup`）
- Node.js 20.19+ / 22.12+，包管理器用 **pnpm**
- macOS：Xcode Command Line Tools；OCR 需要 `brew install tesseract`
  （或用 PaddleOCR sidecar），并授予「屏幕录制」权限
- Windows：MSVC 工具链 + WebView2（Win11 自带）；OCR 建议
  `winget install UB-Mannheim.TesseractOCR`，在「路由与快捷键」里
  把 tesseract 路径指向安装目录（如
  `C:\Program Files\Tesseract-OCR\tesseract.exe`）
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
cargo test --workspace                  # Rust 测试（91 个）
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

## 日志

日志走 `tracing`，输出到 stdout（`pnpm tauri dev` 的终端可见）。
级别用环境变量控制，例如：

```bash
RUST_LOG=lingua=debug pnpm tauri dev
```

约束：API Key、用户划词文本、截图内容不写日志（划词只记录字符数）。

## 验收状态（Phase 5）

| 检查项 | 命令 | 状态 |
|--------|------|------|
| 前端安装 | `pnpm install` | ✅ |
| 前端构建 | `pnpm build` | ✅（main + popup 双入口） |
| Rust 检查 | `cargo check --workspace --all-targets` | ✅ |
| Rust 测试 | `cargo test --workspace` | ✅ 91 passed |
| Lint | `cargo clippy -D warnings` / `pnpm lint` | ✅ |
| 应用链接 | `cargo build --workspace` | ✅ |
| 划词→翻译→Popup 全链路 | `pnpm tauri dev` 手动验证 | ⚠️ 需真机人工验证 |
