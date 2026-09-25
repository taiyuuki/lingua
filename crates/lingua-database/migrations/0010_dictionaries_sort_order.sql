-- 词典手动排序：查询命中顺序（弹窗词典 chips、多词典候选）按此列。
-- 存量词典按创建（导入）顺序初始化；新导入的词典排在末尾。
ALTER TABLE dictionaries ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;

UPDATE dictionaries
   SET sort_order = sub.rn
  FROM (
    SELECT id, ROW_NUMBER() OVER (ORDER BY created_at, id) AS rn FROM dictionaries
  ) AS sub
 WHERE dictionaries.id = sub.id;
