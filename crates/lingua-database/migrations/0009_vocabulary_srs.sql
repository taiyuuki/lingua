-- 生词复习调度（SRS-lite）：到期时间 + 复习间隔。
-- 存量词回填为「已到期、间隔 1 天」——都是查过的词，全部进入复习队列。
ALTER TABLE vocabulary ADD COLUMN due_at TEXT NOT NULL DEFAULT '';
ALTER TABLE vocabulary ADD COLUMN interval_days INTEGER NOT NULL DEFAULT 0;

UPDATE vocabulary
   SET due_at = created_at, interval_days = 1
 WHERE due_at = '';

CREATE INDEX idx_vocab_due ON vocabulary (due_at) WHERE due_at != '';
