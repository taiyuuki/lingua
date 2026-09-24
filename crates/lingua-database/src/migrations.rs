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
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "init_settings",
    sql: include_str!("../migrations/0001_init_settings.sql"),
}];

/// 当前代码支持的最高 schema 版本。
pub fn latest_version() -> u32 {
    MIGRATIONS.last().map_or(0, |migration| migration.version)
}
