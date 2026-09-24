-- MDX 富文本：净化后的原始释义 HTML。
-- TSV 导入与存量 MDX 数据为 NULL，前端回落纯文本列。
ALTER TABLE dictionary_definitions ADD COLUMN definition_html TEXT;
