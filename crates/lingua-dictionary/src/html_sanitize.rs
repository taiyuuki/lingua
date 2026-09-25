//! MDX 释义 HTML 净化器 —— 导入期一次性执行，之后前端按可信内容渲染。
//!
//! [ammonia] 负责标签/属性白名单净化（剥 script / 事件属性 /
//! `javascript:` URL）；词典特有的语义由 `attribute_filter` 追加：
//!
//! * `style` 值走 CSS 属性白名单二级过滤（剥 `url()` / `expression`）；
//! * `<a href>` 只保留 `entry://词头` 词条内链（弹窗跳词）；
//! * `<img src>` / `<link href>` 的资源相对路径改写为
//!   `mdx-res://<词典id>/<规范化路径>`，由后端自定义协议服务。
//!
//! [ammonia]: https://github.com/rust-ammonia/ammonia

use ammonia::{Builder, UrlRelative};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

/// MDD 资源服务协议前缀（`register_uri_scheme_protocol` 注册的同名 scheme）。
pub const RESOURCE_SCHEME: &str = "mdx-res";

/// 词典级净化器：白名单 Builder 只构建一次，逐条 [`clean`] 复用
/// （百万级导入时每条重建 Builder + html5ever 解析会拖慢数十秒）。
pub struct DefinitionSanitizer {
    builder: Builder<'static>,
}

impl DefinitionSanitizer {
    pub fn new(dictionary_id: &str) -> Self {
        // host 固定 localhost，词典 id 与资源路径都编码进路径段：
        // WebView 对自定义 scheme 的 host 字符集有限制（id 可能是中文）。
        let base = format!(
            "{RESOURCE_SCHEME}://localhost/{}/",
            percent_encode_path(dictionary_id),
        );

        let tags: HashSet<&str> = [
            "div", "p", "span", "b", "i", "u", "em", "strong", "small", "big", "sup", "sub",
            "br", "hr", "ul", "ol", "li", "dl", "dt", "dd", "table", "thead", "tbody",
            "tfoot", "tr", "td", "th", "caption", "h1", "h2", "h3", "h4", "h5", "h6",
            "blockquote", "abbr", "font", "a", "img", "link",
        ]
        .into_iter()
        .collect();

        let mut tag_attributes: HashMap<&str, HashSet<&str>> = HashMap::new();
        tag_attributes.insert("a", ["href"].into_iter().collect());
        tag_attributes.insert(
            "img",
            ["src", "alt", "width", "height"].into_iter().collect(),
        );
        tag_attributes.insert(
            "link",
            ["href", "rel", "type"].into_iter().collect(),
        );
        tag_attributes.insert(
            "font",
            ["color", "face", "size"].into_iter().collect(),
        );
        tag_attributes.insert(
            "table",
            ["border", "cellspacing", "cellpadding", "width", "align"]
                .into_iter()
                .collect(),
        );
        tag_attributes.insert(
            "td",
            ["colspan", "rowspan", "align", "valign", "width", "height"]
                .into_iter()
                .collect(),
        );
        tag_attributes.insert(
            "th",
            ["colspan", "rowspan", "align", "valign", "width", "height"]
                .into_iter()
                .collect(),
        );
        tag_attributes.insert("ol", ["start", "type"].into_iter().collect());

        let mut builder = Builder::new();
        builder.tags(tags);
        builder.tag_attributes(tag_attributes);
        builder.generic_attributes(["class", "style"].into_iter().collect());
        // entry（词条内链）与 mdx-res（资源）是自有 scheme；data 允许
        // 行内小图；file 是老词典的伪协议写法（过滤器里剥前缀改写）；
        // 外链 http(s) 放行 scheme 但 a 的 href 过滤只留 entry://。
        builder.url_schemes(
            ["https", "http", "mailto", "data", "file", "entry", RESOURCE_SCHEME]
                .into_iter()
                .collect(),
        );
        builder.url_relative(UrlRelative::PassThrough);
        builder.link_rel(None);
        builder.attribute_filter(move |element, attribute, value| {
            match (element, attribute) {
                (_, "style") => Some(Cow::Owned(sanitize_style(value))),
                ("a", "href") => {
                    // 弹窗里外链点不开浏览器：只保留词条内链。
                    if value.trim().to_ascii_lowercase().starts_with("entry://") {
                        Some(Cow::Borrowed(value))
                    } else {
                        None
                    }
                }
                ("img", "src") | ("link", "href") => rewrite_resource(value, &base),
                _ => Some(Cow::Borrowed(value)),
            }
        });
        Self { builder }
    }

    /// 净化一条释义。
    ///
    /// 快路径：不含 `<` 的纯文本记录不可能构成标签，原样返回
    /// （实体引用交给浏览器照常解码）——纯文本词库（如简明英汉汉英）
    /// 占比接近 100%，可省掉整套 html5ever 解析。
    pub fn clean(&self, html: &str) -> String {
        if !html.contains('<') {
            return html.to_string();
        }
        self.builder.clean(html).to_string()
    }
}

/// 净化一段 MDX 释义 HTML（单条一次性场景用；批量导入请复用
/// [`DefinitionSanitizer`]）。
pub fn sanitize_definition_html(html: &str, dictionary_id: &str) -> String {
    DefinitionSanitizer::new(dictionary_id).clean(html)
}

/// CSS 声明白名单：排版相关的属性放行，定位/行为类剥除。
fn css_property_allowed(property: &str) -> bool {
    const EXACT: [&str; 12] = [
        "color",
        "background",
        "background-color",
        "width",
        "height",
        "vertical-align",
        "display",
        "white-space",
        "word-break",
        "float",
        "clear",
        "line-height",
    ];
    const PREFIXES: [&str; 6] = ["border", "margin", "padding", "font", "text", "list-style"];
    EXACT.contains(&property)
        || PREFIXES.iter().any(|prefix| {
            property == *prefix || property.starts_with(&format!("{prefix}-"))
        })
}

fn sanitize_style(value: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    for declaration in value.split(';') {
        let Some((property, val)) = declaration.split_once(':') else {
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        let val = val.trim();
        if val.is_empty() || !css_property_allowed(&property) {
            continue;
        }
        let lower = val.to_ascii_lowercase();
        if lower.contains("url(") || lower.contains("expression") || lower.contains("javascript")
        {
            continue;
        }
        kept.push(format!("{property}: {val}"));
    }
    kept.join("; ")
}

/// MDD 资源路径规范化：`\`→`/`、去前导分隔符、拒绝 `..` 穿越。
pub fn normalize_resource_path(path: &str) -> Option<String> {
    let trimmed = path.trim().trim_start_matches(['\\', '/']);
    let normalized = trimmed.replace('\\', "/");
    if normalized.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for segment in normalized.split('/') {
        match segment {
            "" | "." => {}
            ".." => return None,
            segment => parts.push(segment),
        }
    }
    Some(parts.join("/"))
}

fn rewrite_resource<'v>(value: &'v str, base: &str) -> Option<Cow<'v, str>> {
    // `file://xxx.gif` 是老词典常见的伪协议写法，剥掉前缀按资源路径处理。
    let value = value.trim().strip_prefix("file://").unwrap_or(value.trim());
    // 已是协议 URL（含行内 data:）原样保留，其余按词典资源路径改写。
    if value.contains("://") || value.starts_with("data:") {
        return Some(Cow::Borrowed(value));
    }
    let normalized = normalize_resource_path(value)?;
    Some(Cow::Owned(format!("{base}{}", percent_encode_path(&normalized))))
}

/// URL 路径段百分号编码（RFC 3986 unreserved + `/` 之外全部转义）。
fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn strips_scripts_events_and_iframes() {
        let out = sanitize_definition_html(
            "<p onclick=\"evil()\">hi</p><script>alert(1)</script><iframe src=\"x\"></iframe>",
            "d1",
        );
        assert!(!out.contains("script"), "{out}");
        assert!(!out.contains("iframe"), "{out}");
        assert!(!out.contains("onclick"), "{out}");
        assert!(out.contains("hi"), "{out}");
    }

    #[test]
    fn style_whitelist_keeps_typography_drops_dangerous() {
        let out = sanitize_definition_html(
            "<span style=\"color:red; position:fixed; background:url(evil)\">x</span>",
            "d1",
        );
        assert!(out.contains("color: red"), "{out}");
        assert!(!out.contains("position"), "{out}");
        assert!(!out.contains("url("), "{out}");
    }

    #[test]
    fn href_keeps_entry_links_drops_external() {
        let out = sanitize_definition_html(
            "<a href=\"entry://hello\">see</a><a href=\"https://x.com\">web</a>",
            "d1",
        );
        assert!(out.contains("entry://hello"), "{out}");
        assert!(!out.contains("https://x.com"), "{out}");
        assert!(out.contains("see"), "{out}");
    }

    #[test]
    fn resource_paths_are_rewritten_to_scheme() {
        let out = sanitize_definition_html(
            "<img src=\"/img/a.png\"><link href=\"\\css\\oald.css\" rel=\"stylesheet\">",
            "d1",
        );
        assert!(out.contains("src=\"mdx-res://localhost/d1/img/a.png\""), "{out}");
        assert!(
            out.contains("href=\"mdx-res://localhost/d1/css/oald.css\""),
            "{out}"
        );
    }

    #[test]
    fn traversal_paths_are_rejected() {
        assert!(normalize_resource_path("../secrets").is_none());
        assert!(normalize_resource_path("a/../../b").is_none());
        assert_eq!(
            normalize_resource_path("\\A\\b//c.png").as_deref(),
            Some("A/b/c.png")
        );
    }

    #[test]
    fn class_attribute_survives() {
        let out = sanitize_definition_html("<span class=\"def chn\">x</span>", "d1");
        assert!(out.contains("class=\"def chn\""), "{out}");
    }

    #[test]
    fn data_image_src_passes_through() {
        let out = sanitize_definition_html("<img src=\"data:image/png;base64,AAAA\">", "d1");
        assert!(out.contains("data:image/png"), "{out}");
    }

    #[test]
    fn file_scheme_prefix_is_treated_as_resource_path() {
        let out = sanitize_definition_html("<img src=\"file://dot.gif\">", "d1");
        assert!(out.contains("mdx-res://localhost/d1/dot.gif"), "{out}");
    }
}
