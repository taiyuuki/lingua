//! 词典 DAO：`dictionaries` / `dictionary_entries` / `dictionary_definitions`。
//!
//! 原始格式的词典数据（TSV / CC-CEDICT / …）由导入层转换为
//! [`DictionaryEntry`] 后写库；查询侧输出统一的 [`DictionaryEntry`]，
//! UI 不感知存储细节。

use crate::{Database, db_err};
use lingua_core::{AppError, AppResult, DictionaryDefinition, DictionaryEntry, Language};
use rusqlite::params;
use serde::Serialize;

/// 词典元信息。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryMeta {
    pub id: String,
    pub name: String,
    pub source_language: Language,
    pub gloss_language: Language,
}

/// 词典元信息 + 词条数量（列表页展示用）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryInfo {
    #[serde(flatten)]
    pub meta: DictionaryMeta,
    pub entry_count: usize,
}

fn json_array_to_strings(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

fn strings_to_json_array(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string())
}

impl Database {
    /// 整体导入一个词典：upsert 元信息 + 逐词条写入，单事务完成。
    /// 语义是**增量**的：同 lemma 词条覆盖旧数据，新文件里没有的词条保留
    /// （支持分批导入）；想要完全替换就换个 dictionary id 或先删除。
    /// 返回写入的词条数。
    ///
    /// 词条 / 释义语句只在事务内准备一次循环复用（此前逐条 `execute`
    /// 每次都重新 prepare，百万级 MDX 导入要多花几十秒）。
    pub fn import_dictionary(
        &self,
        meta: &DictionaryMeta,
        entries: &[DictionaryEntry],
    ) -> AppResult<usize> {
        if meta.id.trim().is_empty() {
            return Err(AppError::InvalidInput(
                "dictionary id is required".to_string(),
            ));
        }
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(db_err)?;
        tx.execute(
            "INSERT INTO dictionaries (id, name, source_lang, gloss_lang)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                source_lang = excluded.source_lang,
                gloss_lang = excluded.gloss_lang",
            params![
                meta.id.trim(),
                meta.name.trim(),
                meta.source_language.code(),
                meta.gloss_language.code()
            ],
        )
        .map_err(db_err)?;

        let dictionary_id = meta.id.trim();
        let mut imported = 0usize;
        {
            // 词条用 upsert（RETURNING 拿稳定 id——注意 upsert 走
            // DO UPDATE 分支时 last_insert_rowid 不更新，返回陈旧值，
            // 必须用 RETURNING），替代原来的「DELETE 旧词条 + INSERT
            // 新词条」：新词典导入省掉一次空转的索引查找，重导时也
            // 不因级联删除重建整行。
            let mut upsert_entry = tx
                .prepare(
                    "INSERT INTO dictionary_entries
                        (dictionary_id, lemma, lemma_lower, language, pronunciations, inflections)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(dictionary_id, lemma_lower) DO UPDATE SET
                        lemma = excluded.lemma,
                        language = excluded.language,
                        pronunciations = excluded.pronunciations,
                        inflections = excluded.inflections
                     RETURNING id",
                )
                .map_err(db_err)?;
            let mut delete_definitions = tx
                .prepare("DELETE FROM dictionary_definitions WHERE entry_id = ?1")
                .map_err(db_err)?;
            let mut insert_definition = tx
                .prepare(
                    "INSERT INTO dictionary_definitions
                        (entry_id, pos, part_of_speech, definition, definition_html,
                         examples, synonyms, antonyms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .map_err(db_err)?;

            // 批内按 lemma 排序：MDX 词条按 record 偏移序到达，对
            // lemma 唯一索引近似乱序插入；排好序后索引写入接近追加，
            // 页缓存与 WAL 的局部性都好得多。
            let mut ordered: Vec<&DictionaryEntry> = entries.iter().collect();
            ordered.sort_by_key(|entry| entry.lemma.to_lowercase());

            for entry in ordered {
                let lemma = entry.lemma.trim();
                if lemma.is_empty() || !entry.has_definitions() {
                    continue;
                }
                let lemma_lower = lemma.to_lowercase();
                let entry_id: i64 = upsert_entry
                    .query_row(
                        params![
                            dictionary_id,
                            lemma,
                            lemma_lower,
                            entry.language.code(),
                            strings_to_json_array(&entry.pronunciations),
                            strings_to_json_array(&entry.inflections),
                        ],
                        |row| row.get(0),
                    )
                    .map_err(db_err)?;
                delete_definitions
                    .execute(params![entry_id])
                    .map_err(db_err)?;

                for (index, definition) in entry.definitions.iter().enumerate() {
                    if definition.definition.trim().is_empty() {
                        continue;
                    }
                    insert_definition
                        .execute(params![
                            entry_id,
                            index as i64,
                            definition.part_of_speech,
                            definition.definition,
                            definition.html,
                            strings_to_json_array(&definition.examples),
                            strings_to_json_array(&definition.synonyms),
                            strings_to_json_array(&definition.antonyms),
                        ])
                        .map_err(db_err)?;
                }
                imported += 1;
            }
        }
        tx.commit().map_err(db_err)?;
        Ok(imported)
    }

    /// 列出全部词典（含词条计数），按创建顺序。
    pub fn list_dictionaries(&self) -> AppResult<Vec<DictionaryInfo>> {
        let conn = self.lock_conn()?;
        let mut statement = conn
            .prepare(
                "SELECT d.id, d.name, d.source_lang, d.gloss_lang,
                        (SELECT COUNT(*) FROM dictionary_entries e WHERE e.dictionary_id = d.id)
                 FROM dictionaries d ORDER BY d.created_at, d.id",
            )
            .map_err(db_err)?;
        let rows = statement
            .query_map([], |row| {
                Ok(DictionaryInfo {
                    meta: DictionaryMeta {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        source_language: Language::parse(&row.get::<_, String>(2)?),
                        gloss_language: Language::parse(&row.get::<_, String>(3)?),
                    },
                    entry_count: row.get::<_, i64>(4)? as usize,
                })
            })
            .map_err(db_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    pub fn delete_dictionary(&self, id: &str) -> AppResult<bool> {
        let conn = self.lock_conn()?;
        let changed = conn
            .execute("DELETE FROM dictionaries WHERE id = ?1", params![id])
            .map_err(db_err)?;
        Ok(changed > 0)
    }

    /// 按词头查询（大小写不敏感），可按词头语言过滤。
    /// 返回的 [`DictionaryEntry::source`] 填充为词典名称（展示用）。
    ///
    /// 精确未命中时按 [`lingua_core::lemma_candidates`] 依次尝试
    /// 词形还原候选（upscaling → upscale）——词典收录的是原形。
    pub fn lookup_entries(
        &self,
        lemma: &str,
        language: Option<Language>,
    ) -> AppResult<Vec<DictionaryEntry>> {
        let entries = self.lookup_entries_exact(lemma, language.clone())?;
        if !entries.is_empty() {
            return Ok(entries);
        }
        for candidate in lingua_core::lemma::lemma_candidates(lemma) {
            let found = self.lookup_entries_exact(&candidate, language.clone())?;
            if !found.is_empty() {
                return Ok(found);
            }
        }
        Ok(entries)
    }

    fn lookup_entries_exact(
        &self,
        lemma: &str,
        language: Option<Language>,
    ) -> AppResult<Vec<DictionaryEntry>> {
        let lemma_lower = lemma.trim().to_lowercase();
        if lemma_lower.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock_conn()?;

        let mut sql = String::from(
            "SELECT e.id, e.lemma, e.language, e.pronunciations, e.inflections,
                    d.gloss_lang, d.name
             FROM dictionary_entries e
             JOIN dictionaries d ON d.id = e.dictionary_id
             WHERE e.lemma_lower = ?1",
        );
        if language.is_some() {
            sql.push_str(" AND d.source_lang = ?2");
        }
        sql.push_str(" ORDER BY d.created_at, d.id");
        let mut statement = conn.prepare(&sql).map_err(db_err)?;

        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(i64, DictionaryEntry)> {
            Ok((
                row.get(0)?,
                DictionaryEntry {
                    lemma: row.get(1)?,
                    language: Language::parse(&row.get::<_, String>(2)?),
                    gloss_language: Language::parse(&row.get::<_, String>(5)?),
                    pronunciations: json_array_to_strings(&row.get::<_, String>(3)?),
                    inflections: json_array_to_strings(&row.get::<_, String>(4)?),
                    source: row.get(6)?,
                    definitions: Vec::new(),
                },
            ))
        };

        let rows: Vec<(i64, DictionaryEntry)> = match language {
            Some(lang) => statement
                .query_map(params![lemma_lower, lang.code()], map_row)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?,
            None => statement
                .query_map(params![lemma_lower], map_row)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?,
        };

        let mut entries = Vec::with_capacity(rows.len());
        for (entry_id, mut entry) in rows {
            let mut definition_statement = conn
                .prepare(
                    "SELECT part_of_speech, definition, definition_html, examples, synonyms, antonyms
                     FROM dictionary_definitions WHERE entry_id = ?1 ORDER BY pos",
                )
                .map_err(db_err)?;
            entry.definitions = definition_statement
                .query_map(params![entry_id], |row| {
                    Ok(DictionaryDefinition {
                        part_of_speech: row.get(0)?,
                        definition: row.get(1)?,
                        html: row.get(2)?,
                        examples: json_array_to_strings(&row.get::<_, String>(3)?),
                        synonyms: json_array_to_strings(&row.get::<_, String>(4)?),
                        antonyms: json_array_to_strings(&row.get::<_, String>(5)?),
                    })
                })
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            entries.push(entry);
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn meta(id: &str) -> DictionaryMeta {
        DictionaryMeta {
            id: id.into(),
            name: format!("Dict {id}"),
            source_language: Language::En,
            gloss_language: Language::Zh,
        }
    }

    fn entry(lemma: &str, definition: &str) -> DictionaryEntry {
        DictionaryEntry {
            lemma: lemma.into(),
            language: Language::En,
            gloss_language: Language::Zh,
            pronunciations: vec!["/test/".into()],
            inflections: vec![],
            source: "my-dict".into(),
            definitions: vec![DictionaryDefinition {
                part_of_speech: Some("noun".into()),
                definition: definition.into(),
                html: None,
                examples: vec![format!("{lemma} example")],
                synonyms: vec![],
                antonyms: vec![],
            }],
        }
    }

    #[test]
    fn import_lookup_and_delete_roundtrip() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();

        let imported = db
            .import_dictionary(
                &meta("my-dict"),
                &[entry("Hello", "你好"), entry("world", "世界")],
            )
            .unwrap();
        assert_eq!(imported, 2);

        let infos = db.list_dictionaries().unwrap();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].meta.id, "my-dict");
        assert_eq!(infos[0].entry_count, 2);

        let results = db.lookup_entries("HELLO", Some(Language::En)).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].lemma, "Hello");
        assert_eq!(results[0].definitions[0].definition, "你好");
        // source 填词典名称（展示用）。
        assert_eq!(results[0].source, "Dict my-dict");

        // 语言不匹配时查不到。
        assert!(
            db.lookup_entries("hello", Some(Language::Ja))
                .unwrap()
                .is_empty()
        );
        // 不给语言时跨词典命中。
        assert_eq!(db.lookup_entries("hello", None).unwrap().len(), 1);

        // 重新导入同 lemma 覆盖该词条（增量语义，未出现的词条保留）。
        db.import_dictionary(&meta("my-dict"), &[entry("hello", "您好")])
            .unwrap();
        let infos = db.list_dictionaries().unwrap();
        assert_eq!(infos[0].entry_count, 2);
        let overwritten = db.lookup_entries("hello", None).unwrap();
        assert_eq!(overwritten[0].definitions.len(), 1, "旧释义不应残留");
        assert_eq!(overwritten[0].definitions[0].definition, "您好");
        assert_eq!(
            db.lookup_entries("world", None).unwrap()[0].definitions[0].definition,
            "世界"
        );

        assert!(db.delete_dictionary("my-dict").unwrap());
        assert!(db.lookup_entries("hello", None).unwrap().is_empty());
        assert!(!db.delete_dictionary("my-dict").unwrap());
    }

    #[test]
    fn inflected_form_falls_back_to_lemma() {
        // 词典只收原形，变化形态靠词形还原候选兜底命中。
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        db.import_dictionary(&meta("my-dict"), &[entry("upscale", "提升")])
            .unwrap();
        let inflected = db.lookup_entries("upscaling", Some(Language::En)).unwrap();
        assert_eq!(inflected.len(), 1);
        assert_eq!(inflected[0].lemma, "upscale");
        assert_eq!(inflected[0].definitions[0].definition, "提升");
    }

    #[test]
    fn multiple_dictionaries_merge_in_lookup() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        db.import_dictionary(&meta("dict-a"), &[entry("cat", "猫")])
            .unwrap();
        let mut meta_b = meta("dict-b");
        meta_b.source_language = Language::En;
        db.import_dictionary(&meta_b, &[entry("cat", "猫科动物")])
            .unwrap();

        let results = db.lookup_entries("cat", Some(Language::En)).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].source, "Dict dict-a");
        assert_eq!(results[1].definitions[0].definition, "猫科动物");
    }

    #[test]
    fn import_skips_blank_entries() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        let mut blank = entry("ok", "有效");
        blank.definitions.clear();
        let imported = db
            .import_dictionary(&meta("d"), &[blank, entry("fine", "好的")])
            .unwrap();
        assert_eq!(imported, 1);
    }
}
