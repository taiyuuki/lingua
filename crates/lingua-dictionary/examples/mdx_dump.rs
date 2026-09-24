//! MDX 解析器手动验证工具：`cargo run -p lingua-dictionary --example mdx_dump -- <file.mdx>`
//!
//! 打印 header 摘要、词条组数，并完整走一遍词条迭代（统计重定向条目），
//! 最后给出前几条词条的纯文本预览。

use lingua_dictionary::mdx::{MdxEntry, MdxFile};

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "usage: mdx_dump <file.mdx>".to_string());
    let file = std::path::PathBuf::from(&path);
    match MdxFile::open(&file) {
        Ok(dict) => {
            println!("file        : {path}");
            println!("title       : {:?}", dict.title());
            println!("entry groups: {}", dict.entry_count());

            let mut total = 0usize;
            let mut redirects = 0usize;
            let mut previews: Vec<MdxEntry> = Vec::new();
            for entry in dict.entries() {
                if entry.is_redirect() {
                    redirects += 1;
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
