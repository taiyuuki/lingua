//! 英文词形还原候选生成（词典查词兜底）。
//!
//! 词典收录的是原形（lemma），划词常拿到变化形态（upscaling /
//! running / stopped…）。这里用一组保守的后缀规则生成候选原形，
//! 调用方按序精确查词典直到命中——不做语言学完备推导，漏掉时
//! 原词本身就是第一个候选。
//!
//! 只处理 ASCII 字母词；其它文字（CJK 等）返回空候选。

/// 生成词形还原候选（小写、去重、按可能性排序）。
/// 输入应已 trim；候选不包含原词本身（调用方先查原词）。
pub fn lemma_candidates(word: &str) -> Vec<String> {
    let word = word.trim().to_lowercase();
    let stripped = word
        .strip_suffix("'s")
        .or_else(|| word.strip_suffix("s'"))
        .map(str::to_string);
    let word = stripped.clone().unwrap_or(word);
    if word.len() < 3 || !word.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Vec::new();
    }

    let mut candidates: Vec<String> = Vec::new();
    let mut push = |value: String| {
        if value.len() >= 3 && !candidates.contains(&value) {
            candidates.push(value);
        }
    };
    // 所有格剥离本身就是候选（cat's → cat）
    if let Some(base) = stripped {
        push(base);
    }

    // 复数 / 第三人称单数
    if let Some(stem) = word.strip_suffix("ies") {
        push(format!("{stem}y")); // studies → study
    }
    if let Some(stem) = word.strip_suffix("es") {
        push(stem.to_string()); // boxes → box, watches → watch
    }
    if let Some(stem) = word.strip_suffix('s') {
        // bus / is / ss 结尾不做（会切出无意义词干）
        if !word.ends_with("ss") && !word.ends_with("us") && !word.ends_with("is") {
            push(stem.to_string()); // cats → cat
        }
    }

    // 进行时 / 过去分词
    if let Some(stem) = word.strip_suffix("ing") {
        push(stem.to_string());
        push(format!("{stem}e")); // making → make；upscaling → upscale
        if let Some(undoubled) = undoubled(stem) {
            push(undoubled); // running → run
        }
    }
    // 过去式 / 过去分词
    if let Some(stem) = word.strip_suffix("ed") {
        push(stem.to_string());
        push(format!("{stem}e")); // decoded → decode
        if let Some(undoubled) = undoubled(stem) {
            push(undoubled); // stopped → stop
        }
    }

    // 副词
    if let Some(stem) = word.strip_suffix("ly") {
        push(stem.to_string()); // quickly → quick
    }
    // 比较级 / 最高级
    if let Some(stem) = word.strip_suffix("est") {
        push(i_to_y(stem));
        push(stem.to_string());
    }
    if let Some(stem) = word.strip_suffix("er") {
        push(i_to_y(stem)); // happier → happy
        push(stem.to_string());
    }
    // 名词化
    if let Some(stem) = word.strip_suffix("ation") {
        push(format!("{stem}ate")); // creation → create
    }

    candidates
}

fn i_to_y(stem: &str) -> String {
    stem.strip_suffix('i').map_or_else(|| stem.to_string(), |head| format!("{head}y"))
}

/// 双写辅音还原：running→run、shopping→shop（词干 ≥4 且末两位
/// 为相同辅音时剥末字符）。
fn undoubled(stem: &str) -> Option<String> {
    let bytes = stem.as_bytes();
    if bytes.len() >= 4
        && bytes[bytes.len() - 1] == bytes[bytes.len() - 2]
        && !matches!(bytes[bytes.len() - 1], b'a' | b'e' | b'i' | b'o' | b'u')
    {
        Some(stem[..stem.len() - 1].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn contains(word: &str, candidates: &[String]) -> bool {
        candidates.iter().any(|candidate| candidate == word)
    }

    #[test]
    fn generates_expected_lemmas() {
        assert!(contains("upscale", &lemma_candidates("upscaling")), "{:?}",
            lemma_candidates("upscaling"));
        assert!(contains("make", &lemma_candidates("making")));
        assert!(contains("run", &lemma_candidates("running")));
        assert!(contains("shop", &lemma_candidates("shopping")));
        assert!(contains("stop", &lemma_candidates("stopped")));
        assert!(contains("decode", &lemma_candidates("decoded")));
        assert!(contains("study", &lemma_candidates("studies")));
        assert!(contains("cat", &lemma_candidates("cats")));
        assert!(contains("quick", &lemma_candidates("quickly")));
        assert!(contains("happy", &lemma_candidates("happier")));
        assert!(contains("create", &lemma_candidates("creation")));
        assert!(contains("cat", &lemma_candidates("cat's")));
    }

    #[test]
    fn skips_short_non_alpha_and_cjk() {
        assert!(lemma_candidates("go").is_empty());
        assert!(lemma_candidates("さくら").is_empty());
        assert!(lemma_candidates("hello-1").is_empty());
    }
}
