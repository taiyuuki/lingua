-- v11: 翻译历史（划词与截图翻译共用一张表，entry_kind 区分入口）。
--
-- 纯追加日志：只记录「哪段文本 → 译成了什么」，不承载用户整理
-- 数据（笔记/标签/掌握度在生词本）。写入侧自动裁剪到最近 N 条。

CREATE TABLE translation_history (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    source_text       TEXT NOT NULL,
    translated_text   TEXT NOT NULL,
    detected_language TEXT,             -- 检测/识别出的源语言（BCP-47 基础码）
    target_language   TEXT NOT NULL,    -- 译入语（BCP-47 基础码）
    model             TEXT,             -- AI 模型/Provider 标签；词典命中为 NULL
    entry_kind        TEXT NOT NULL DEFAULT 'selection', -- selection | ocr
    dictionary_hit    INTEGER NOT NULL DEFAULT 0,
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_history_created ON translation_history (created_at DESC);
