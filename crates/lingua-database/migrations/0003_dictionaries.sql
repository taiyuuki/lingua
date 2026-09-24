-- v3: 词典（dictionaries / dictionary_entries / dictionary_definitions）。
--
-- * dictionaries：词典元信息；source_lang 是词头语言，gloss_lang 是释义语言。
-- * dictionary_entries：词头；lemma_lower 是大小写归一后的查询键。
-- * dictionary_definitions：释义，按 pos（序号）排序；examples / synonyms /
--   antonyms 是 JSON 字符串数组。
-- * 级联删除：删除词典会级联删除其词条与释义。

CREATE TABLE dictionaries (
    id          TEXT PRIMARY KEY,        -- 词典标识，如 'my-en-zh'
    name        TEXT NOT NULL,
    source_lang TEXT NOT NULL,
    gloss_lang  TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE dictionary_entries (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    dictionary_id  TEXT NOT NULL REFERENCES dictionaries(id) ON DELETE CASCADE,
    lemma          TEXT NOT NULL,
    lemma_lower    TEXT NOT NULL,
    language       TEXT NOT NULL,
    pronunciations TEXT NOT NULL DEFAULT '[]',  -- JSON 数组
    inflections    TEXT NOT NULL DEFAULT '[]',  -- JSON 数组
    UNIQUE (dictionary_id, lemma_lower)
);

CREATE INDEX idx_entries_lookup ON dictionary_entries (dictionary_id, lemma_lower);
CREATE INDEX idx_entries_lemma ON dictionary_entries (lemma_lower);

CREATE TABLE dictionary_definitions (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_id       INTEGER NOT NULL REFERENCES dictionary_entries(id) ON DELETE CASCADE,
    pos            INTEGER NOT NULL,             -- 释义排序
    part_of_speech TEXT,
    definition     TEXT NOT NULL,
    examples       TEXT NOT NULL DEFAULT '[]',   -- JSON 数组
    synonyms       TEXT NOT NULL DEFAULT '[]',   -- JSON 数组
    antonyms       TEXT NOT NULL DEFAULT '[]'    -- JSON 数组
);

CREATE INDEX idx_definitions_entry ON dictionary_definitions (entry_id, pos);
