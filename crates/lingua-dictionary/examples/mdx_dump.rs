//! MDX/MDD 解析器手动验证工具：
//! `cargo run -p lingua-dictionary --example mdx_dump -- <file.mdx|file.mdd>`
//!
//! .mdx：打印 header 摘要、词条组数，完整走一遍词条迭代（统计重定向），
//! 给出前几条词条的纯文本预览；.mdd：解包到临时目录并列出资源。

use lingua_dictionary::mdx::{MdxEntry, MdxFile};

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "usage: mdx_dump <file.mdx|.mdd>".to_string());
    let file = std::path::PathBuf::from(&path);
    if file.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("mdd")) {
        let out = std::env::temp_dir().join("lingua-mdd-dump");
        let _ = std::fs::remove_dir_all(&out);
        match lingua_dictionary::mdx::extract_mdd_resources(&file, &out) {
            Ok(count) => {
                println!("extracted {count} resources to {}", out.display());
                let mut listed = 0usize;
                list_files(&out, &mut listed);
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

fn list_files(dir: &std::path::Path, listed: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            list_files(&path, listed);
        } else if *listed < 10 {
            println!("  {}", path.strip_prefix(dir).unwrap_or(&path).display());
            *listed += 1;
        }
    }
}
