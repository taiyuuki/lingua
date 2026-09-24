-- v5: providers 增加 disable_thinking —— 请求体附带
-- {"thinking": {"type": "disabled"}}，关闭智谱 GLM 等模型默认的
-- 深度思考（翻译类任务不需要推理，开着容易超时）。

ALTER TABLE providers ADD COLUMN disable_thinking INTEGER NOT NULL DEFAULT 0;
