-- v4: 生词本（vocabulary / vocabulary_reviews）。
--
-- 与词典表完全分离：词典是「词 → 释义」的静态数据，
-- 生词本是「用户 ↔ 词」的关系数据（查询次数、笔记、标签、掌握度）。
-- SRS（间隔重复）算法后续阶段实现，vocabulary_reviews 先留数据轨迹。

CREATE TABLE vocabulary (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    word          TEXT NOT NULL,
    word_lower    TEXT NOT NULL,             -- 归一查询键
    language      TEXT NOT NULL,             -- 词头语言（BCP-47 基础码）
    translation   TEXT,                      -- 最近一次译文
    notes         TEXT NOT NULL DEFAULT '',
    tags          TEXT NOT NULL DEFAULT '[]',-- JSON 字符串数组
    mastery       INTEGER NOT NULL DEFAULT 0,-- 0=new 1=learning 2=mastered
    lookup_count  INTEGER NOT NULL DEFAULT 0,
    review_count  INTEGER NOT NULL DEFAULT 0,
    source        TEXT NOT NULL DEFAULT '',  -- 词典 id / 'ai' / 'manual'
    examples      TEXT NOT NULL DEFAULT '[]',-- JSON 字符串数组
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_seen_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (word_lower, language)
);

CREATE INDEX idx_vocab_seen ON vocabulary (last_seen_at DESC);

CREATE TABLE vocabulary_reviews (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    vocab_id    INTEGER NOT NULL REFERENCES vocabulary(id) ON DELETE CASCADE,
    result      INTEGER NOT NULL DEFAULT 0, -- 0=again 1=good 2=easy（SRS 数据字段）
    reviewed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_reviews_vocab ON vocabulary_reviews (vocab_id, reviewed_at);
