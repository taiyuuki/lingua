//! Prompt 模板：Prompt 与业务代码分离。
//!
//! * 模板文件放在 crate 的 `prompts/` 目录，`include_str!` 编译期内嵌，
//!   二进制自包含；
//! * 占位符用 `{{name}}` 语法；
//! * 未来允许用户自定义 Prompt 时，只需在加载处增加覆盖逻辑，
//!   渲染器不变。

use std::collections::HashMap;

/// 一组配对模板（system + user）。
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    system: String,
    user: String,
}

impl PromptTemplate {
    pub fn from_parts(system: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            system: system.into(),
            user: user.into(),
        }
    }

    /// 渲染 system / user 两条消息内容；未出现的占位符渲染为空串。
    pub fn render(&self, vars: &HashMap<String, String>) -> (String, String) {
        (render_str(&self.system, vars), render_str(&self.user, vars))
    }
}

/// 极简 `{{name}}` 替换（不做嵌套/转义；Prompt 场景足够）。
/// 未闭合的 `{{` 原样保留。
fn render_str(template: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"{{") {
            if let Some(end) = template[index + 2..].find("}}") {
                let name = template[index + 2..index + 2 + end].trim();
                out.push_str(vars.get(name).map(String::as_str).unwrap_or(""));
                index += 2 + end + 2;
                continue;
            }
            out.push_str("{{");
            index += 2;
            continue;
        }
        let ch = template[index..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

pub(crate) fn vars_from_pairs(pairs: &[(&str, String)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn renders_named_placeholders() {
        let template =
            PromptTemplate::from_parts("You are a {{role}}.", "Translate to {{target}}:\n{{text}}");
        let vars = vars_from_pairs(&[
            ("role", "translator".into()),
            ("target", "zh".into()),
            ("text", "hello\nworld".into()),
        ]);
        let (system, user) = template.render(&vars);
        assert_eq!(system, "You are a translator.");
        assert_eq!(user, "Translate to zh:\nhello\nworld");
    }

    #[test]
    fn missing_and_unclosed_placeholders_are_safe() {
        let template = PromptTemplate::from_parts("{{missing}} {{broken", "x");
        let (system, user) = template.render(&HashMap::new());
        assert_eq!(system, " {{broken");
        assert_eq!(user, "x");
    }

    #[test]
    fn multibyte_text_survives_rendering() {
        let template = PromptTemplate::from_parts("译成{{lang}}：{{text}}", "");
        let vars = vars_from_pairs(&[("lang", "中文".into()), ("text", " café ".into())]);
        let (system, _) = template.render(&vars);
        assert_eq!(system, "译成中文： café ");
    }
}
