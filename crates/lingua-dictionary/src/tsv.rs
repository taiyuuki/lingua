//! TSV 词典导入器 —— 通用格式转换器。
//!
//! 行格式（UTF-8，TAB 分隔；`#` 开头为注释；同一 lemma 多行合并为一个词条）：
//!
//! ```text
//! lemma <TAB> pronunciation <TAB> part_of_speech <TAB> definition [<TAB> examples [<TAB> synonyms [<TAB> antonyms]]]
//! ```
//!
//! * `examples` / `synonyms` / `antonyms` 多个值用 `" / "` 分隔；
//! * 公开词典数据（CC-CEDICT / JMdict / ECDICT…）可先用脚本转成该格式；
//! * 解析以行为单位容错：坏行跳过并计数，不会让整个导入失败。

use lingua_core::{DictionaryDefinition, DictionaryEntry, Language};
use std::collections::HashMap;

const FIELD_SEPARATOR: &str = " / ";

/// 解析结果：词条列表 + 被跳过的行数（空行、注释、坏行）。
pub struct ParsedTsv {
    pub entries: Vec<DictionaryEntry>,
    pub skipped_lines: usize,
}

/// 解析 TSV 词典文本。
///
/// `source_language` / `gloss_language` / `source` 由调用方提供
/// （来自导入表单），解析器只负责结构映射。
pub fn parse_tsv(
    text: &str,
    source_language: Language,
    gloss_language: Language,
    source: &str,
) -> ParsedTsv {
    struct Partial {
        entry: DictionaryEntry,
        seen_pronunciations: HashMap<String, ()>,
    }

    let mut partials: Vec<Partial> = Vec::new();
    let mut index_by_lemma: HashMap<String, usize> = HashMap::new();
    let mut skipped = 0usize;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            skipped += 1;
            continue;
        }
        let fields: Vec<&str> = line.split('\t').map(str::trim).collect();
        if fields.len() < 4 {
            skipped += 1;
            continue;
        }
        let lemma = fields[0];
        let pronunciation = fields[1];
        let part_of_speech = fields[2];
        let definition = fields[3];
        if lemma.is_empty() || definition.is_empty() {
            skipped += 1;
            continue;
        }
        let lemma = lemma.to_string();
        let pronunciation = pronunciation.to_string();
        let part_of_speech = part_of_speech.to_string();
        let definition = definition.to_string();

        let examples = fields.get(4).map(|v| split_list(v)).unwrap_or_default();
        let synonyms = fields.get(5).map(|v| split_list(v)).unwrap_or_default();
        let antonyms = fields.get(6).map(|v| split_list(v)).unwrap_or_default();

        let definition = DictionaryDefinition {
            part_of_speech: (!part_of_speech.is_empty()).then_some(part_of_speech),
            definition,
            examples,
            synonyms,
            antonyms,
        };

        match index_by_lemma.get(&lemma.to_lowercase()) {
            Some(&index) => {
                let partial = &mut partials[index];
                partial.entry.definitions.push(definition);
                if !pronunciation.is_empty()
                    && !partial.seen_pronunciations.contains_key(&pronunciation)
                {
                    partial
                        .seen_pronunciations
                        .insert(pronunciation.clone(), ());
                    partial.entry.pronunciations.push(pronunciation);
                }
            }
            None => {
                index_by_lemma.insert(lemma.to_lowercase(), partials.len());
                let mut pronunciations = Vec::new();
                if !pronunciation.is_empty() {
                    pronunciations.push(pronunciation.clone());
                }
                let mut seen = HashMap::new();
                if !pronunciation.is_empty() {
                    seen.insert(pronunciation, ());
                }
                partials.push(Partial {
                    entry: DictionaryEntry {
                        lemma,
                        language: source_language.clone(),
                        gloss_language: gloss_language.clone(),
                        pronunciations,
                        inflections: Vec::new(),
                        source: source.to_string(),
                        definitions: vec![definition],
                    },
                    seen_pronunciations: seen,
                });
            }
        }
    }

    ParsedTsv {
        entries: partials.into_iter().map(|partial| partial.entry).collect(),
        skipped_lines: skipped,
    }
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(FIELD_SEPARATOR)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn parse(text: &str) -> ParsedTsv {
        parse_tsv(text, Language::En, Language::Zh, "test")
    }

    #[test]
    fn parses_basic_rows_and_merges_by_lemma() {
        let parsed = parse(
            "ephemeral\t/ɪˈfem.ər.əl/\tadj\t短暂的\tan ephemeral moment / 朝露\ttransient\tpermanent\n\
             ephemeral\t/ɪˈfem.ər.əl/\tnoun\t蜉蝣\n\
             hello\t\tnoun\t你好",
        );
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.skipped_lines, 0);

        let ephemeral = &parsed.entries[0];
        assert_eq!(ephemeral.lemma, "ephemeral");
        assert_eq!(ephemeral.pronunciations, vec!["/ɪˈfem.ər.əl/"]);
        assert_eq!(ephemeral.definitions.len(), 2);
        assert_eq!(ephemeral.definitions[0].examples.len(), 2);
        assert_eq!(ephemeral.definitions[0].synonyms, vec!["transient"]);
        assert_eq!(ephemeral.definitions[0].antonyms, vec!["permanent"]);
        assert_eq!(ephemeral.definitions[1].definition, "蜉蝣");
    }

    #[test]
    fn skips_comments_blank_and_bad_lines() {
        let parsed = parse("# 注释\n\n\thello\nhello\t\tnoun\t你好");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.skipped_lines, 3);
    }

    #[test]
    fn missing_definition_row_is_skipped() {
        let parsed = parse("word\t\tadj");
        assert_eq!(parsed.entries.len(), 0);
        assert_eq!(parsed.skipped_lines, 1);
    }

    #[test]
    fn empty_pos_becomes_none() {
        let parsed = parse("word\t\t\t定义");
        assert_eq!(parsed.entries[0].definitions[0].part_of_speech, None);
    }
}
