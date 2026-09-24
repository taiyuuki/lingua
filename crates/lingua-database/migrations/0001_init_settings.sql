-- v1: 应用设置（JSON 键值对）。
-- settings 是唯一允许自由结构的表；providers、vocabulary 等业务表
-- 必须使用明确的列 schema，在后续 migration 中添加。

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,              -- JSON 编码
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_settings_updated_at ON settings (updated_at);
