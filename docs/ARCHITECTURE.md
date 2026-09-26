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
| `lingua-platform` | `SelectionProvider` + `SelectionService`；macOS 实现（AX 选区 + 剪贴板 fallback + 光标/截图：`screencapture` CLI + 屏幕录制权限 FFI）；Windows 实现（UI Automation TextPattern 选区 + 剪贴板 fallback（SendInput Ctrl+C）+ 光标（GetCursorPos + 显示器 DPI 换算）+ GDI 截图）；Linux 明确占位 | ✅ macOS Phase 1/5、Windows |
| `lingua-ai` | `AIProvider` trait（含 `vision`）、`OpenAiCompatibleProvider`（reqwest/rustls；chat/SSE/多模态 image_url）、`AIRouter`（AiTask → Provider 路由 + 默认回退 + Vision 能力分派）、`ProviderConfig`（api_key 不序列化） | ✅ Phase 1/2/6 |
| `lingua-translation` | Translation Engine：单词 → 词典优先（释义语言须匹配目标语言），未命中/短语/句子 → AI；`prompts/translate/` 模板内嵌；AI JSON 解析失败优雅降级；同语言换向（`{{same_language_rule}}` 变量注入）；自定义 system prompt 覆盖（`PromptOverride`）；`StudyAnalyzer` 学习分析（`prompts/study/`，解析失败降级 notes） | ✅ Phase 1/3/9 |
| `lingua-dictionary` | `DictionaryProvider` trait、SQLite 词典实现、TSV 导入器（CC-CEDICT 等公开词典可先转 TSV 再导入） | ✅ Phase 3 |
| `lingua-ocr` | `OcrProvider` trait、Tesseract CLI 实现、HTTP sidecar 实现（PaddleOCR 契约）、`VisionOcrProvider`（vision 能力 AIProvider 的高级 fallback，输出复用 `OcrResult`） | ✅ Phase 5/6 |
| `lingua-database` | SQLite（rusqlite bundled）、migration runner、settings KV、providers / dictionaries / vocabulary / translation_history 表 DAO | ✅ Phase 0/1/3/4/8 |
| `lingua-secrets` | `SecretStore` trait + keyring 实现（macOS Keychain / Windows Credential Manager / Linux Secret Service）；API Key 明文只存在这里 | ✅ Phase 1 |
| `lingua` (src-tauri) | Tauri 壳：commands、全局快捷键、翻译 pipeline（经 Router）、Popup、Provider 管理/连接测试/应用设置命令 | ✅ Phase 1/2 |

后续按阶段新增（不为形式提前建空 crate）：

- 已落地：流式输出（SSE，`translate_stream` + `popup://delta` 事件）、
  `lingua-ocr`（Phase 5/6）、翻译历史（migration 0011 + 主窗口历史页）、
  学习分析（`AiTask::Study` + `StudyAnalyzer`）、生词本 Anki 导出、
  Prompt 自定义。
- **Phase 10**：Wayland 分治（GNOME/KDE）、多屏拼接截图、OCR 原位置
  覆盖翻译（bbox 数据已保留）。

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
全局快捷键（划词 alt+t / 截图 alt+s，settings 可配）
    ↓ tauri-plugin-global-shortcut（Rust 侧注册，无需前端权限）
pipeline::run_translation_pipeline
    ↓ 1. SelectionService.capture_first()
    │      macOS Accessibility (AXSelectedText)
    │        ↓ 失败/不支持/无选区
    │      剪贴板 fallback（备份 → Cmd+C → 轮询 600ms → 恢复）
    │      权限缺失 → AppError::Permission（不污染剪贴板，直接报错）
    ↓ 2. Popup 弹出（固定于主显示器中央，显示加载态，抢焦点以支持 Esc）
    ↓ 3. TranslationEngine
    │      prompts/translate 模板渲染 → OpenAI-compatible POST /chat/completions
    │      api_key 从 OS 密钥库读取；配置从 providers 表读取
    ↓ 4. Popup 切换结果态（事件 popup://state + 快照双通道）
         Esc / 失焦隐藏；复制译文按钮
```

## 4. 窗口系统

| 窗口 | label | 入口 | 状态 |
|------|-------|------|------|
| 主窗口 | `main` | `tauri.conf.json` | ✅ Phase 0；关闭 = 隐藏到托盘（划词/截图继续可用），退出走托盘菜单 |
| Popup | `popup` | `popup.html`（Vite 多入口） | ✅ 无边框、置顶（工具栏图钉可切换）、skip taskbar、失焦自动隐藏、光标所在显示器居中（边界钳制）；Esc 关闭 |
| OCR 选区 | `ocr-selection` | Phase 5 | 计划：全屏透明 overlay，独立实现注意 Wayland 限制 |
| Settings / Vocabulary | 复用 main 路由或独立窗口 | Phase 2/4 | 待定 |

Capabilities（`src-tauri/capabilities/default.json`）当前只放行
`main` / `popup` + `core:default`；新增窗口或插件权限时同步维护。

Windows 焦点注记：WebView2 是独立的子 HWND 树，点击弹窗内容会把
真实焦点转给子窗口并触发主窗口 `Focused(false)`——「失焦隐藏」
不能直接信这个事件。现以前台根窗口判定
（`lingua_platform::window_in_foreground`）区分「点击内部」与
「点击外部」，前者再由 `popup::handle_focus_lost` 的轮询 watcher
（150ms）盯到真正失焦才隐藏（父窗口不会再收到第二次失焦事件）。

## 5. 平台差异备忘（Phase 1/5/10 的输入）

- **全局快捷键**：用 `tauri-plugin-global-shortcut`（官方插件），不要自写平台钩子。
- **划词**：macOS 需要用户在系统设置授予 **辅助功能（Accessibility）** 权限；
  截屏需要 **屏幕录制** 权限。权限检测/引导应走 `AppError::Permission`。
  剪贴板 fallback 必须备份并恢复剪贴板。
  Windows 无系统级开关：UI Automation / SendInput / GDI 截图开箱可用；
  唯一边界是 UIPI —— 普通权限进程读不到管理员权限窗口的选区，
  表现为 `NoSelection`（温和提示，无需权限引导）；GDI 截屏对 DRM
  保护内容只能得到黑块（平台限制）。剪贴板 fallback 在模拟 Ctrl+C
  前必须等快捷键的修饰键物理释放：SendInput 与物理按键状态是叠加
  关系，Alt 未松开时发出的是 Alt+Ctrl+C，应用不会复制
  （macOS CGEventPost 自带完整修饰键掩码，无此问题）。
- **Wayland**：无可靠的全局选区坐标与全局快捷键方案，Popup 定位能力受限；
  Phase 10 按桌面环境（GNOME/KDE）逐个处理，X11 下能力完整。
- **托盘/自启**：已完成 —— 托盘（tauri 内建 `tray-icon`）常驻，
  菜单含显示主窗口 / 截图翻译 / 开机自启（CheckMenuItem）/
  退出；左键单击显示主窗。关闭主窗口只是隐藏（`CloseRequested`
  拦截）。自启用 `tauri-plugin-autostart`，托盘勾选与设置页开关
  经 `set_autostart` / 托盘事件双向同步。

## 6. Phase 1 已知限制

1. **剪贴板 fallback 只备份文本**：富文本/图片剪贴板内容在 fallback 后
   只能恢复为文本；且「选区内容恰好等于剪贴板原文」时无法检测变化。
2. **Popup 定位**：光标所在显示器内居中并做边界钳制，多显示器下
   跟随光标（每次新触发重定位，Loading → Result 原地刷新不跳位）。
   未使用工作区（居中通常不会压任务栏，极端情况可能遮挡）、
   Wayland 下光标坐标受限，均留 Phase 10。
3. **Linux 划词未实现**：注册的是明确的 unsupported 占位
   Provider，报「平台暂不支持」而不是假装工作。Windows 划词已实现
  （UIA + 剪贴板 fallback），见 §5 平台差异备忘的 UIPI 限制。

（初版的「未真机验证 / 同语言照翻 / 非流式输出」三条已分别由
真机验证、`translation.swap_language` 同语言换向、`translate_stream`
流式输出解决。）

## 7. Phase 2 已知限制

1. 多 Provider 管理已完成：增删改、每条连接测试（发 16 token 探测请求并测延迟）、
   默认 Provider / 翻译任务路由（存 settings 表）、删除时自动清理密钥与路由。
2. 流式输出（SSE）已落地：`translate_stream` 走纯文本流式 prompt，
   增量经 `popup://delta` 事件 + `latest_streamed` 快照双通道下发。
3. Prompt 自定义已开放（Phase 9 一并落地，见 §11）。
4. 路由粒度：Translate / Vision / Study 三个任务；后续新任务在
   `AiTask` 枚举上继续扩展。


## 8. Phase 3 已知限制

1. 词典已完成：三表 schema（dictionaries / dictionary_entries /
   dictionary_definitions，migration 0003）、`DictionaryProvider` trait +
   SQLite 实现、TSV 导入器（行级容错、同 lemma 合并、单事务增量导入）、
   Popup 词典视图（音标 / 词性 / 释义 / 例句）。
2. 导入已接文件选择器（`tauri-plugin-dialog`，随打包阶段引入）；
   路径输入框保留可手填。MDX（MDict）词库已支持：无加密 v1.2 / v2.0
   与固定密钥加密（Encrypted=2，ripemd128 派生密钥，离线可解）
   （`lingua-dictionary::mdx`，LZO / zlib 解压、GBK / UTF-16 / UTF-8
   解码、同释义多词头合并为词形变化、`@@@LINK=` 重定向跳过）。
   释义双份存储：纯文本列 + 导入期经 ammonia 白名单净化的富文本
   HTML（migration 0008）；同目录同名 .mdd 资源包（CSS / 图片 /
   音频）与同目录散装样式/字体（OALD10 的 OALD8C.css 布局）统一
   收集到 `app_data/mdx/<id>/`，经 `mdx-res://` 自定义协议服务
   （Windows 子资源用 `http://mdx-res.localhost` 形态）；弹窗富文本
   渲染、`entry://` 内链跳词与 `#锚点` 定位、`sound://` 发音链接
   原生 Audio 播放（词典 JS 一律不执行）。
   大词库分批写库并经 `dictionary://import-progress` 事件上报进度。
   绑定用户注册信息的加密（Encrypted 含 1）不支持。
   CC-CEDICT / JMdict 原生解析器未做，现阶段用脚本转 TSV
   （格式见导入表单提示）。
3. 词典路由对「单词与词组」生效（`classify_text` 启发式：句末标点
   判为整句才绕过词典；词组未命中静默回退 AI）。
4. 释义语言必须等于目标语言才采用词典结果，跨多目标语言需导入
   对应方向的词典。

## 8b. Phase 4 已知限制（生词本）

1. 已完成：migration 0004（`vocabulary` / `vocabulary_reviews`）、
   upsert 语义（自动记录累计 `lookup_count`，手动收藏不计；用户数据
   永不被自动写入覆盖）、筛选（搜索/标签/掌握度）、Popup「★ 收藏」、
   生词本管理 UI（编辑笔记/标签/掌握度、删除）。
2. SRS-lite 复习已落地（生词本 2.0）：到期队列（migration 0009 的
   `due_at` / `interval_days`）+ 翻卡复习 + 三档评分（忘记/记得/简单）
   + 间隔阶梯 `[1,2,4,7,15,30,60]` 天重排期。
3. 生词本暂与词典 DAO 同居 `lingua-database`；Phase 9 学习分析走
   `lingua-translation::study`，未再拆 vocabulary crate。
4. Anki 导出已完成（TSV + Anki 原生导入指令头，无需插件）；
   AI 生成学习卡片由学习分析承担（Popup 学习卡片），.apkg 打包未做。


## 9. Phase 5 已知限制（OCR）

1. 已完成：`lingua-core::ocr` 契约（`OcrImage` + `OcrResult`，保留
   text/bbox/confidence/language/blocks/lines/words 粒度）；
   `lingua-ocr` crate（`OcrProvider` trait + Tesseract CLI 实现 +
   HTTP sidecar 实现）；`lingua-platform::capture`（macOS 走系统
   `screencapture` CLI，屏幕录制权限 FFI 预检/请求，截图即读即删；
   Windows 走 GDI BitBlt——物理像素、无需权限，输出前强制
   不透明以免 PNG 在 webview 里透明渲染）；
   **快照式选区 overlay**（`ocr.html` 入口）：start 时先截主显示器
   整屏存入内存会话，overlay 显示这张冻结截图，用户在图上框选
   （拖拽、Esc/右键取消、误点不退出）；裁剪缩放比按「截图像素 /
   前端上报的 overlay CSS 视口」自校准（HiDPI / 显示器缩放下
   不依赖 Rust 侧对 monitor 尺寸的推断），按比例换算像素坐标，
   `crop_image`（image crate，PNG）在快照上裁剪后送 OCR。
   不依赖窗口透明 / macOSPrivateApi，遮罩也不会被截进图。
   pipeline：裁剪 → OCR → 走现有 Router/Engine 翻译 → Popup。
2. **未真机人工验证**（macOS 与 Windows 均需 `pnpm tauri dev` 走一遍）；
   macOS 需要授予「屏幕录制」权限（权限预检 + 系统授权框已接），
   Windows 无需权限。
3. Tesseract 中文识别需额外语言包（`brew install tesseract-lang`），
   且 Phase 5 未传 `-l` 语言参数（自动检测由翻译端处理）。
4. PaddleOCR 以 sidecar 形式接入（`scripts/paddleocr_server.py`，
   需自行安装 paddleocr/paddlepaddle；部署文档见 `docs/PADDLEOCR.md`）；
   ONNX 内嵌推理未做。
5. 图片预处理（二值化/放大）为空操作；「OCR → 原位置覆盖翻译」
   依赖的 bbox 数据已保留，实现留到后续阶段。
6. 多显示器：快照与 overlay 均跟随光标所在显示器（截图与选区坐标
   同屏对齐，混合 DPI 也不串屏）；跨多屏拼接成一张大截图未做；
   OCR 历史随翻译历史统一记录（Phase 8，`entry_kind = "ocr"`）。
   截图入口默认 `alt+s` 全局快捷键
   （主窗口按钮入口的光标必然在主窗口所在屏，只能截那一块）。

## 10. Phase 6：Vision AI（视觉大模型识别）

已完成（真机验证待做）：

1. `AiTask::Vision` 路由 + Provider capabilities.vision 分派：路由 /
   默认 Provider 不具备视觉能力时自动改派任一有能力的 Provider；
   OpenAI-compatible 实现走 `image_url` base64 多模态消息
   （`extra_body` 照常合并）。
2. OCR 后处理兜底：**自动**——空结果或块平均置信度 < 0.6 时经 Router
   派给 Vision 重识别；**手动**——Popup 工具栏「用 Vision 重新识别」
   按钮（tesseract 常见故障是高置信度的错，自动阈值触发不了），
   对保留的选区截图（`AppState.ocr_retry`，划词触发时清除）直接走
   Vision 并重翻译。Vision 失败/为空**不阻断**，回退原始结果。
   **可观测性**：识别来源（引擎 / 路径 / 置信度 / 兜底未生效原因，
   `AppState.ocr_source`）在 Popup 结果态以标签展示，用于区分
   「识别错了」与「Vision 没触发」。
   输出复用 `OcrResult` 契约——bbox 置零、置信度 0，不伪造几何数据；
   严格 JSON（text/language）解析失败降级纯文本。
3. 设置里为 Vision 任务单独指定 Provider（「视觉识别 Provider」，
   未设置 = 自动挑选）；Provider 表单新增能力勾选（对话/翻译/视觉）。
4. `scripts/paddleocr_server.py` 部署文档化（`docs/PADDLEOCR.md`）。

（定位：Vision LLM 不替代普通 OCR，只做高级 fallback —— 需求原话。）

## 11. Phase 8/9：翻译历史 · 学习分析 · 导出 · Prompt 自定义

已完成（真机验证待做）：

1. **翻译历史**：migration 0011 `translation_history`（划词 / OCR 共用，
   `entry_kind` 区分入口）。pipeline 与 OCR 链路在结果态后旁路写入
   （`record_history_task`，失败只告警不阻断）；写入侧自动裁剪到最近
   5000 条。主窗口「历史」页：原文/译文搜索、来源筛选、分页、单条
   删除、一键清空。
2. **同语言智能换向**：设置 `translation.swap_language`（默认 `en`，
   可关）。引擎在两条 system prompt 的 `{{same_language_rule}}` 占位符
   注入换向指令，「文本已是目标语言」由模型阅读时判断（不引入本地
   语言检测依赖）；换向语言 == 目标语言视为不换向。
3. **学习分析（Phase 9）**：`AiTask::Study` 路由（路由 → 默认回退），
   设置页可单独指派 Provider。`lingua-translation::study::StudyAnalyzer`
   + `prompts/study/`：单词深析（词性 / 义项 / 词根词缀 / 搭配 / 近义
   辨析 / 记忆点）与句子解构（结构 / 关键短语 / 复述 / 仿写例句），
   释义语言跟随目标语言。严格 JSON（camelCase 键）解析失败降级
   `notes` 全文，绝不因格式偏差报错。Popup 工具栏「学习分析」按钮
   以整面板展示（Esc 先关面板再关弹窗）；带词典词头与已有译文上下文。
4. **生词本 Anki 导出**：`export_vocabulary_anki` 命令生成带 Anki 导入
   指令头的 TSV（`#separator:tab` / `#columns` / `#tags column:5`），
   卡片 = Word / Meaning / Examples / Notes / 标签+语言；字段内
   tab/换行折叠为空格保证行结构。生词本页保存对话框导出。
5. **Prompt 自定义**：`TranslationEngine::with_prompt_override` 覆盖
   system 模板（`prompt.translate_system` / `prompt.translate_stream`，
   空白 = 内置）。user 模板（含原文位置）不可覆盖；变量照常渲染。
   设置页「翻译 Prompt（高级）」双 textarea + 变量提示 + 恢复内置。

后续候选（按需排）：Wayland（Phase 10）、多屏拼接截图、OCR 原位置
覆盖翻译（bbox 数据已保留）、.apkg 打包导出、学习统计报表。
（图标与文件选择器、Windows 打包 nsis/msi、托盘与开机自启已完成：
`pnpm tauri build` 产出 `target/release/bundle/` 下两种安装包。）

## 12. 词典发音库（AudioLib 形态 .mdd）

**场景**：牛津 9 这类词库正文里发音链接（`snd://…spx`）指向的音频不在
配对 .mdd 里，需要独立的「发音库」兜底——即 MDict 圈流通的 AudioLib
. mdd（如英式美式发音库：`word.spx` 美音 + `word_Uk.spx` 英音，24.8 万
条、Encrypted=2）。NHK 式纯数字命名（`SPX\0001.spx`）无法按词头定位，
只能作为配对 .mdd 随词典导入（已支持）。

**架构决策**（区别于配对 .mdd 的整体解包）：

- **原件收编 + 索引入库 + 随机读取**。发音库音频不落成几十万个小文件：
  .mdd 复制到 `app_data/audiolibs/<id>.mdd`（id 前缀 `al-`，按文件名
  哈希生成，同文件重导覆盖）；词头→音频索引进 SQLite（migration 0012：
  `audio_libraries` / `audio_keys`，PK = 库+词头+口音，`word_lower`
  索引）；`lingua-dictionary::audio::MddResourceFile` 只解析 header /
  key 区 / record 块布局（几 MB，seek 式读取，不扫几百 MB 数据区），
  `read_resource` 按 key 定位偏移 → 解压单个 record 块 → 切片返回。
- **口音提取**（`extract_audio_entries`，导入期一次性）：识别裸名
  `word.spx`（默认美音）/ `word_Uk.spx`、口音目录 `us\` / `uk\`、
  口音+同词序号后缀 `word__gb_1` / `_speak_1_us_3`；纯数字名与非音频
  扩展名跳过。词头入库为小写。
- **服务复用 `mdx-res://`**：`lookup_word_audio` 返回
  `mdx-res://localhost/<库id>/<资源路径>`；协议处理器先查词典资源
  目录、未命中再查发音库句柄缓存（`AppState.audio_files`，setup 预载、
  导入/删除命令同步增删），`.spx` 照常就地转 WAV。前端
  `platformResourceUrl` 做与富文本相同的 Windows WebView2 形态转换。
- **前端**：词条面板（Popup 词典区 / 词卡 / 查词页共用）词头旁
  🔊英 / 🔊美 按钮，按当前 lemma 查 `lookup_word_audio`，命中才显示；
  词典内嵌 `sound://` 喇叭不受影响（更准的义项级发音仍优先）。
  管理入口在词典管理页「发音库」卡片（添加 / 删除 / 口音覆盖展示）。

**已知限制**：多口音取「最先导入的库」的命中；牛津 9 的 `snd://`
前缀净化改写与「按引用文件名跨库解析」未做（P2 候选）；日语等
非英/美口音体系未建模。
