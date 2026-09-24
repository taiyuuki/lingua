-- idx_entries_lookup (dictionary_id, lemma_lower) 与 UNIQUE 约束生成的
-- 自动索引完全同列，纯冗余：词条每次写入要多维护一棵 B-tree，
-- 百万级词典导入时约三分之一写库时间耗在这上面。
-- 保留 idx_entries_lemma（lemma_lower 左前缀，跨词典词头查询用）。

DROP INDEX IF EXISTS idx_entries_lookup;
