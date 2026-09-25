//! MDX 导入性能基准：`cargo run -p lingua-dictionary --release --example mdx_bench -- <file.mdx>`
//!
//! 复刻 `import_dictionary_mdx` 命令的核心路径（分批入库 + 计数），
//! 写入临时 SQLite，最后做一次词头查询验证，并输出各阶段耗时。

#![allow(clippy::expect_used)]

use lingua_core::Language;
use lingua_dictionary::mdx::MdxFile;
use lingua_database::Database;
use std::time::Instant;

const BATCH: usize = 5_000;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "usage: mdx_bench <file.mdx>".to_string());
    let lemma = std::env::args().nth(2).unwrap_or_else(|| "test".to_string());

    let started = Instant::now();
    let dict = MdxFile::open(std::path::Path::new(&path)).expect("open mdx");
    let total = dict.entry_count();
    println!("parse: {:?} ({total} groups)", started.elapsed());

    let db_path = std::env::temp_dir().join("lingua-mdx-bench.sqlite3");
    let _ = std::fs::remove_file(&db_path);
    // MEMDB=1 时写内存库：分离磁盘 I/O 与 CPU 成本。
    let database = if std::env::var_os("MEMDB").is_some() {
        Database::open_in_memory().expect("open mem db")
    } else {
        Database::open(&db_path).expect("open db")
    };
    database.migrate().expect("migrate");
    let meta = lingua_database::DictionaryMeta {
        id: "bench".into(),
        name: "bench".into(),
        source_language: Language::Ja,
        gloss_language: Language::Zh,
    };

    let import_started = Instant::now();
    let mut raw_batch = Vec::with_capacity(BATCH);
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for entry in dict.entries() {
        if entry.is_redirect() {
            skipped += 1;
            continue;
        }
        raw_batch.push(entry);
        if raw_batch.len() >= BATCH {
            let batch = lingua_dictionary::mdx::convert_entries_parallel(
                &raw_batch,
                &Language::Ja,
                &Language::Zh,
                "bench",
            )
            .expect("convert batch");
            imported += database.import_dictionary(&meta, &batch).expect("import batch");
            raw_batch.clear();
        }
    }
    if !raw_batch.is_empty() {
        let batch = lingua_dictionary::mdx::convert_entries_parallel(
            &raw_batch,
            &Language::Ja,
            &Language::Zh,
            "bench",
        )
        .expect("convert last batch");
        imported += database.import_dictionary(&meta, &batch).expect("import last batch");
    }
    println!(
        "import: {:?} ({imported} written, {skipped} redirects skipped)",
        import_started.elapsed()
    );

    let lookup_started = Instant::now();
    let hits = database
        .lookup_entries(&lemma, None)
        .expect("lookup");
    println!(
        "lookup `{lemma}`: {:?} ({} hits)",
        lookup_started.elapsed(),
        hits.len()
    );
    println!("total: {:?}, db: {}", started.elapsed(), db_path.display());
}
