//! MDX/MDD 解析器手动验证工具：
//! `cargo run -p lingua-dictionary --example mdx_dump [--raw|--list] -- <file>`
//!
//! .mdx：打印 header 摘要、词条组数与纯文本预览；`--raw` 额外打印
//! 前 3 条的原始 HTML（看样式/音频引用形式用）。
//! .mdd：默认只列资源清单（路径数 + 扩展名分布）；`--extract`
//! 才真正解包到临时目录。

use lingua_dictionary::mdx::{MdxEntry, MdxFile};
use std::collections::BTreeMap;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let raw = args.iter().any(|a| a == "--raw");
    let extract = args.iter().any(|a| a == "--extract");
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "usage: mdx_dump [--raw|--extract] <file>".to_string());
    let file = std::path::PathBuf::from(&path);
    if file.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("mdd")) {
        match lingua_dictionary::mdx::list_mdd_resources(&file) {
            Ok(paths) => {
                println!("mdd 资源 {} 个", paths.len());
                let mut extensions: BTreeMap<String, usize> = BTreeMap::new();
                for p in &paths {
                    let ext = p.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                    *extensions.entry(ext).or_default() += 1;
                }
                println!("扩展名分布: {extensions:?}");
                for p in paths.iter().take(15) {
                    println!("  {p}");
                }
                if extract {
                    let out = std::env::temp_dir().join("lingua-mdd-dump");
                    let _ = std::fs::remove_dir_all(&out);
                    match lingua_dictionary::mdx::extract_mdd_resources(&file, &out) {
                        Ok(count) => println!("extracted {count} to {}", out.display()),
                        Err(error) => {
                            println!("EXTRACT ERROR: {error}");
                            std::process::exit(1);
                        }
                    }
                }
            }
            Err(error) => {
                println!("ERROR: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    match MdxFile::open(&file) {
        Ok(dict) => {
            println!("file        : {path}");
            println!("title       : {:?}", dict.title());
            println!("entry groups: {}", dict.entry_count());

            let mut total = 0usize;
            let mut redirects = 0usize;
            let mut previews: Vec<MdxEntry> = Vec::new();
            let mut raws = 0usize;
            for entry in dict.entries() {
                if entry.is_redirect() {
                    redirects += 1;
                    continue;
                }
                if raw && raws < 3 {
                    raws += 1;
                    println!("RAW [{}]:\n{}", entry.lemma, entry.text);
                    continue;
                }
                if previews.len() < 5 {
                    previews.push(entry);
                }
                total += 1;
            }
            println!("entries     : {total} (redirects skipped: {redirects})");
            for entry in previews {
                let preview: String = lingua_dictionary::mdx::html_to_plain_text(&entry.text)
                    .chars()
                    .take(120)
                    .collect();
                println!("  [{}] {}", entry.lemma, preview.replace('\n', " ⏎ "));
            }
        }
        Err(error) => {
            println!("ERROR: {error}");
            std::process::exit(1);
        }
    }
}
