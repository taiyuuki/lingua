//! 嵌入式 migration 列表。
//!
//! 规则：
//!
//! * 每个 migration 是一个不可变的 SQL 文件，**已合入的文件禁止修改**；
//!   改 schema 永远靠新增文件。
//! * 版本号从 1 开始连续递增，文件名 `{version:04}_{name}.sql`。
//! * 每个 migration 在独立事务中执行，失败整体回滚。

/// 一条 schema migration。
#[derive(Debug)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

/// 全部 migration，按版本号升序。
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init_settings",
        sql: include_str!("../migrations/0001_init_settings.sql"),
    },
    Migration {
        version: 2,
        name: "providers",
        sql: include_str!("../migrations/0002_providers.sql"),
    },
    Migration {
        version: 3,
        name: "dictionaries",
        sql: include_str!("../migrations/0003_dictionaries.sql"),
    },
    Migration {
        version: 4,
        name: "vocabulary",
        sql: include_str!("../migrations/0004_vocabulary.sql"),
    },
    Migration {
        version: 5,
        name: "provider_thinking",
        sql: include_str!("../migrations/0005_provider_thinking.sql"),
    },
    Migration {
        version: 6,
        name: "provider_extra_body",
        sql: include_str!("../migrations/0006_provider_extra_body.sql"),
    },
    Migration {
        version: 7,
        name: "drop_redundant_entries_index",
        sql: include_str!("../migrations/0007_drop_redundant_entries_index.sql"),
    },
    Migration {
        version: 8,
        name: "definition_html",
        sql: include_str!("../migrations/0008_definition_html.sql"),
    },
    Migration {
        version: 9,
        name: "vocabulary_srs",
        sql: include_str!("../migrations/0009_vocabulary_srs.sql"),
    },
    Migration {
        version: 10,
        name: "dictionaries_sort_order",
        sql: include_str!("../migrations/0010_dictionaries_sort_order.sql"),
    },
    Migration {
        version: 11,
        name: "translation_history",
        sql: include_str!("../migrations/0011_translation_history.sql"),
    },
];

/// 当前代码支持的最高 schema 版本。
pub fn latest_version() -> u32 {
    MIGRATIONS.last().map_or(0, |migration| migration.version)
}
