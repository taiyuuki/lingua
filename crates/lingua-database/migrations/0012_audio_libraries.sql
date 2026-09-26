-- v12: 发音库（独立 .mdd 音频包，按词头定位音频）。
--
-- 发音库与词典解耦：一个库覆盖所有词典的查词发音，点喇叭时按
-- `audio_keys.word_lower` 查词头，口音（uk/us）由导入期从 mdd 的
-- key 命名提取。.mdd 原件收编在 app_data/audiolibs/<id>.mdd，
-- 音频按需随机读取，不入库、不解包。

CREATE TABLE audio_libraries (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    file_path   TEXT NOT NULL,    -- 收编后的 .mdd 绝对路径（app_data/audiolibs/<id>.mdd）
    entry_count INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE audio_keys (
    library_id    TEXT NOT NULL REFERENCES audio_libraries(id) ON DELETE CASCADE,
    word_lower    TEXT NOT NULL,
    accent        TEXT NOT NULL,  -- uk | us
    resource_path TEXT NOT NULL,  -- mdd 内规范化资源路径（us/hello.mp3）
    PRIMARY KEY (library_id, word_lower, accent)
);

CREATE INDEX idx_audio_keys_word ON audio_keys (word_lower);
