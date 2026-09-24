-- v6: providers 的厂商扩展参数改为通用 extra_body（JSON 对象文本），
-- 取代 v5 引入的硬编码 disable_thinking 开关。
-- 例：智谱 GLM 关闭深度思考 → {"thinking":{"type":"disabled"}}

ALTER TABLE providers DROP COLUMN disable_thinking;
ALTER TABLE providers ADD COLUMN extra_body TEXT;
