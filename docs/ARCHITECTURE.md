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
| `lingua-core` | 领域模型：`AppError`（含 `NoSelection`）、`Language`、`SelectedText`、`TranslationRequest/Result`，IPC 序列化契约（camelCase） | ✅ Phase 0/1 |
| `lingua-platform` | `SelectionProvider` + `SelectionService`；macOS 实现（AX 选区 + 剪贴板 fallback + 光标/截图：`screencapture` CLI + 屏幕录制权限 FFI）；Windows/Linux 明确占位 | ✅ macOS Phase 1/5 |
| `lingua-ai` | `AIProvider` trait、`OpenAiCompatibleProvider`（reqwest/rustls）、`AIRouter`（AiTask → Provider 路由 + 默认回退）、`ProviderConfig`（api_key 不序列化） | ✅ Phase 1/2 |
| `lingua-translation` | Translation Engine：单词 → 词典优先（释义语言须匹配目标语言），未命中/短语/句子 → AI；`prompts/translate/` 模板内嵌；AI JSON 解析失败优雅降级 | ✅ Phase 1/3 |
| `lingua-dictionary` | `DictionaryProvider` trait、SQLite 词典实现、TSV 导入器（CC-CEDICT 等公开词典可先转 TSV 再导入） | ✅ Phase 3 |
| `lingua-database` | SQLite（rusqlite bundled）、migration runner、settings KV、providers / dictionaries / vocabulary 表 DAO | ✅ Phase 0/1/3/4 |
| `lingua-secrets` | `SecretStore` trait + keyring 实现（macOS Keychain / Windows Credential Manager / Linux Secret Service）；API Key 明文只存在这里 | ✅ Phase 1 |
| `lingua` (src-tauri) | Tauri 壳：commands、全局快捷键、翻译 pipeline（经 Router）、Popup、Provider 管理/连接测试/应用设置命令 | ✅ Phase 1/2 |

后续按阶段新增（不为形式提前建空 crate）：

- **流式输出（SSE）**：Phase 2 未做，随 Phase 3+ 的体验优化一起落地。
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
| **keyring v4（默认平台 store）** | v4 默认特性即覆盖三平台原生凭据库；v1 兼容层 API 简单稳定。明文密钥只进 OS 密钥库，`api_key_ref`（`provider/{id}`）可安全入库。 |
| **reqwest 0.13 默认 rustls** | 0.13 起 rustls 是默认 TLS 后端，跨平台无 OpenSSL 依赖；`system-proxy` 默认开启，兼容企业代理。 |
| **错误即契约** | `AppError`（thiserror）`#[non_exhaustive]`，序列化为 `{kind, message}`；前端 `describeError` 按 kind 映射文案。workspace lints 对 `unwrap/expect/panic` 告警（测试模块显式豁免）。 |
| **IPC 统一 camelCase** | 所有跨边界类型 `#[serde(rename_all = "camelCase")]`，与 TS 接口一致。 |
| **不用 UI 组件库** | 需求要求轻量桌面风格；Popup 对体积与启动速度敏感。当前用设计令牌 + 手写样式；若后续 Settings/词汇管理页表单复杂度上升，再评估引入（会先说明理由）。 |
| **edition 2024 / clippy 收紧** | rustc 1.97，workspace 统一 lints（unsafe deny，unwrap/expect/panic warn）。macOS FFI 是唯一 `unsafe` 区域（`lingua-platform/src/macos/ffi.rs`）。 |
| **CSP 暂为 null** | Phase 0 无外部资源加载；接入远程内容前收紧并补充。 |
| **翻译要求模型输出严格 JSON** | Prompt（`prompts/translate/system.md`）约束 JSON schema；解析失败时降级为整段文本当译文，绝不因模型格式偏差让翻译失败。用户自定义 Prompt 在 Phase 2+ 开放。 |

## 3.5 Phase 1 完整链路

```
全局快捷键 (alt+t，settings 可配)
    ↓ tauri-plugin-global-shortcut（Rust 侧注册，无需前端权限）
pipeline::run_translation_pipeline
    ↓ 1. SelectionService.capture_first()
    │      macOS Accessibility (AXSelectedText)
    │        ↓ 失败/不支持/无选区
    │      剪贴板 fallback（备份 → Cmd+C → 轮询 600ms → 恢复）
    │      权限缺失 → AppError::Permission（不污染剪贴板，直接报错）
    ↓ 2. Popup 弹出（光标附近定位，显示加载态，抢焦点以支持 Esc）
    ↓ 3. TranslationEngine
    │      prompts/translate 模板渲染 → OpenAI-compatible POST /chat/completions
    │      api_key 从 OS 密钥库读取；配置从 providers 表读取
    ↓ 4. Popup 切换结果态（事件 popup://state + 快照双通道）
         Esc / 失焦隐藏；复制译文按钮
```

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

## 6. Phase 1 已知限制

1. **未做真机人工验证**：全链路（快捷键 → 划词 → 翻译 → Popup）需要
   `pnpm tauri dev` 在真机上手动验证；辅助功能权限需用户手动授予。
2. **剪贴板 fallback 只备份文本**：富文本/图片剪贴板内容在 fallback 后
   只能恢复为文本；且「选区内容恰好等于剪贴板原文」时无法检测变化。
3. **Popup 定位未做屏幕边界钳制**：光标在屏幕右/下边缘时 Popup 可能
   越界（Phase 10 处理，含多显示器）。
4. **同语言翻译**：源语言 == 目标语言时仍会照翻（Prompt 已要求在
   explanation 里说明），智能换向留给 Phase 2 Router。
5. **非流式输出**：长文本翻译期间 Popup 只有加载态，Phase 2 加 SSE。
6. **Windows / Linux 划词未实现**：注册的是明确的 unsupported 占位
   Provider，报「平台暂不支持」而不是假装工作。

## 7. Phase 2 已知限制

1. 多 Provider 管理已完成：增删改、每条连接测试（发 16 token 探测请求并测延迟）、
   默认 Provider / 翻译任务路由（存 settings 表）、删除时自动清理密钥与路由。
2. 流式输出（SSE）未做：长文本翻译期间 Popup 只有加载态，随 Phase 3+ 落地。
3. Prompt 自定义入口未做（内置模板结构已就绪，覆盖逻辑留到词典/学习阶段一起）。
4. 路由粒度目前只有 Translate 一个任务；Vision（Phase 6）/学习分析（Phase 9）
   接入时在 `AiTask` 枚举上扩展即可。


## 8. Phase 3 已知限制

1. 词典已完成：三表 schema（dictionaries / dictionary_entries /
   dictionary_definitions，migration 0003）、`DictionaryProvider` trait +
   SQLite 实现、TSV 导入器（行级容错、同 lemma 合并、单事务增量导入）、
   Popup 词典视图（音标 / 词性 / 释义 / 例句）。
2. 导入用「文件路径输入」而非文件选择器（需要 `tauri-plugin-dialog`，
   随打包阶段一起引入）；CC-CEDICT / JMdict 原生解析器未做，
   现阶段用脚本转 TSV（格式见导入表单提示）。
3. 词典路由只对「单词」生效（`classify_text` 启发式：CJK ≤4 字、
   其它 ≤1 词）；生词/词组的词典短路是后续优化。
4. 释义语言必须等于目标语言才采用词典结果，跨多目标语言需导入
   对应方向的词典。

## 8b. Phase 4 已知限制（生词本）

1. 已完成：migration 0004（`vocabulary` / `vocabulary_reviews`）、
   upsert 语义（自动记录累计 `lookup_count`，手动收藏不计；用户数据
   永不被自动写入覆盖）、筛选（搜索/标签/掌握度）、Popup「★ 收藏」、
   生词本管理 UI（编辑笔记/标签/掌握度、删除）。复习打点 DAO 就绪。
2. 复习 UI 与 SRS 算法未做（Phase 9）；`vocabulary_reviews` 只留数据轨迹。
3. 生词本暂与词典 DAO 同居 `lingua-database`；Phase 9 学习系统引入
   真正领域逻辑时再评估拆 `lingua-vocabulary` crate。
4. Anki 导出 / AI 生成学习卡片未做（后续阶段）。


## 9. Phase 5 已知限制（OCR）

1. 已完成：`lingua-core::ocr` 契约（`OcrImage` + `OcrResult`，保留
   text/bbox/confidence/language/blocks/lines/words 粒度）；
   `lingua-ocr` crate（`OcrProvider` trait + Tesseract CLI 实现 +
   HTTP sidecar 实现）；`lingua-platform::capture`（macOS 走系统
   `screencapture` CLI，屏幕录制权限 FFI 预检/请求，截图即读即删）；
   全屏透明选区 overlay（`ocr.html` 入口，拖拽框选、Esc/右键取消、
   误点不退出）；OCR pipeline（隐藏 overlay → 截图 → OCR → 走现有
   Router/Engine 翻译 → Popup）。
2. **未真机人工验证**：选区 overlay 的透明度依赖
   `macOSPrivateApi`（个人使用无碍，App Store 分发不可用）；
   需要授予「屏幕录制」权限。
3. Tesseract 中文识别需额外语言包（`brew install tesseract-lang`），
   且 Phase 5 未传 `-l` 语言参数（自动检测由翻译端处理）。
4. PaddleOCR 以 sidecar 形式接入（`scripts/paddleocr_server.py`，
   需自行安装 paddleocr/paddlepaddle）；ONNX 内嵌推理未做。
5. 图片预处理（二值化/放大）为空操作；「OCR → 原位置覆盖翻译」
   依赖的 bbox 数据已保留，实现留到后续阶段。
6. 多显示器：overlay 目前只铺主显示器；`ocr_history` 留到 Phase 8。

## 10. Phase 6 待办（下一步）

按原规划进入 **Vision AI（视觉大模型识别）**：

1. `AiTask::Vision` 路由 + Provider capabilities.vision 分派
   （OpenAI-compatible 的 `image_url` base64 消息格式）。
2. OCR 后处理兜底：低置信度 / 空结果时提示「用 Vision 重新识别」；
   Vision 结果仍输出结构化 blocks（复用 `OcrResult` 契约）。
3. 设置里为 Vision 任务单独指定 Provider。
4. `scripts/paddleocr_server.py` 的部署文档化。
（注：Vision LLM 不替代普通 OCR，只做高级 fallback —— 需求原话。）
