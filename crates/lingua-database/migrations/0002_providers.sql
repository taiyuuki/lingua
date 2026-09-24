-- v2: AI Provider 配置表。
-- 注意：api_key 永远不进数据库，存放在 OS 密钥库；
-- `api_key_ref` 只保存密钥在密钥库中的引用（account 名）。

CREATE TABLE providers (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL,
    kind           TEXT NOT NULL,            -- openai_compatible | openai
    base_url       TEXT NOT NULL,
    model          TEXT,
    organization   TEXT,
    timeout_secs   INTEGER NOT NULL DEFAULT 30,
    can_chat       INTEGER NOT NULL DEFAULT 1,
    can_translate  INTEGER NOT NULL DEFAULT 0,
    can_vision     INTEGER NOT NULL DEFAULT 0,
    api_key_ref    TEXT,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
