//! Runs the embedded migrations, after forgiving line-ending-only checksum drift.
//!
//! `sqlx::migrate!` checksums each migration's raw bytes at compile time. Before
//! `.gitattributes` pinned `backend/migrations/*.sql` to LF (#69), a Windows
//! checkout with `core.autocrlf=true` built them as CRLF, so a dev DB can hold
//! CRLF checksums where the build now carries LF ones, or the other way round.
//! sqlx would refuse to start with `VersionMismatch`. A stored checksum that
//! matches the migration with its line endings flipped is the same SQL, so it is
//! rewritten to the embedded checksum before sqlx checks it. Any other mismatch
//! is a real edit to an applied migration, and sqlx still rejects it.
//!
//! **Two phases (#207).** `migrations/*.sql` run first, then startup syncs
//! `content/*.toml` into the static tables, then `migrations/post-content/*.sql`
//! run ([`run_post_content`]). A migration that writes world rows naming
//! content (a node of a resource type, a station of a station type) goes in
//! the second set, where that content exists. The rules for it are in
//! `migrations/post-content/README.md`.

use std::collections::HashMap;

use sha2::{Digest, Sha384};
use sqlx::migrate::Migration;
use sqlx::PgPool;

/// Where [`run_post_content`] records what it has applied. sqlx 0.8 can't
/// give a second migrator its own table, and sharing `_sqlx_migrations` would
/// make each set see the other's versions as missing.
const POST_CONTENT_TABLE: &str = "_post_content_migrations";

/// The advisory lock two servers starting together take round the
/// post-content migrations, so only one applies them.
const POST_CONTENT_LOCK: i64 = 0x0207_c0de;

pub async fn run(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let migrator = sqlx::migrate!("./migrations");

    let table: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations')::text")
            .fetch_one(pool)
            .await?;
    if table.is_some() {
        let applied: Vec<(i64, Vec<u8>)> =
            sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations")
                .fetch_all(pool)
                .await?;
        for (version, stored) in applied {
            let Some(m) = migrator.iter().find(|m| m.version == version) else {
                continue;
            };
            if stored.as_slice() == &*m.checksum {
                continue;
            }
            if is_line_ending_twin(&stored, m) {
                sqlx::query("UPDATE _sqlx_migrations SET checksum = $1 WHERE version = $2")
                    .bind(&*m.checksum)
                    .bind(version)
                    .execute(pool)
                    .await?;
                tracing::info!(
                    "Migration {version} was applied with other line endings; checksum updated"
                );
            }
        }
    }

    migrator.run(pool).await?;
    Ok(())
}

/// Runs the post-content migrations, `migrations/post-content/*.sql`, in
/// version order, each once. Call it after the content sync. Returns how many
/// it applied.
pub async fn run_post_content(pool: &PgPool) -> Result<usize, Box<dyn std::error::Error>> {
    apply_post_content(pool, sqlx::migrate!("./migrations/post-content").iter().as_slice()).await
}

/// Applies the `migrations` that [`POST_CONTENT_TABLE`] doesn't hold yet, in
/// one transaction, so a failure leaves the database as it was. An applied
/// migration whose SQL has changed (other than its line endings) is an error,
/// and so is one recorded as applied that this build doesn't have.
async fn apply_post_content(pool: &PgPool, migrations: &[Migration]) -> Result<usize, Box<dyn std::error::Error>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(POST_CONTENT_LOCK).execute(&mut *tx).await?;
    sqlx::raw_sql(&format!(
        "CREATE TABLE IF NOT EXISTS {POST_CONTENT_TABLE} (
             version BIGINT PRIMARY KEY,
             description TEXT NOT NULL,
             checksum BYTEA NOT NULL,
             installed_on TIMESTAMPTZ NOT NULL DEFAULT NOW()
         )"
    ))
    .execute(&mut *tx)
    .await?;
    let applied: HashMap<i64, Vec<u8>> =
        sqlx::query_as(&format!("SELECT version, checksum FROM {POST_CONTENT_TABLE}"))
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .collect();
    if let Some(version) = applied.keys().find(|&&v| !migrations.iter().any(|m| m.version == v)) {
        return Err(format!("post-content migration {version} was applied, but this build doesn't have it").into());
    }

    let mut ran = 0;
    for m in migrations.iter().filter(|m| !m.migration_type.is_down_migration()) {
        match applied.get(&m.version) {
            Some(stored) if stored.as_slice() == &*m.checksum => {}
            Some(stored) if is_line_ending_twin(stored, m) => {
                sqlx::query(&format!("UPDATE {POST_CONTENT_TABLE} SET checksum = $1 WHERE version = $2"))
                    .bind(&*m.checksum)
                    .bind(m.version)
                    .execute(&mut *tx)
                    .await?;
            }
            Some(_) => {
                return Err(format!("post-content migration {} was edited after it was applied", m.version).into());
            }
            None => {
                if m.no_tx {
                    return Err(format!("post-content migration {}: -- no-transaction isn't supported", m.version).into());
                }
                sqlx::raw_sql(&m.sql).execute(&mut *tx).await?;
                sqlx::query(&format!(
                    "INSERT INTO {POST_CONTENT_TABLE} (version, description, checksum) VALUES ($1, $2, $3)"
                ))
                .bind(m.version)
                .bind(&*m.description)
                .bind(&*m.checksum)
                .execute(&mut *tx)
                .await?;
                tracing::info!("Post-content migration {} applied: {}", m.version, m.description);
                ran += 1;
            }
        }
    }
    tx.commit().await?;
    Ok(ran)
}

/// Whether `stored` is the checksum of `m`'s SQL with its line endings flipped.
fn is_line_ending_twin(stored: &[u8], m: &Migration) -> bool {
    stored == Sha384::digest(line_ending_twin(&m.sql).as_bytes()).as_slice()
}

/// The same SQL with the other line endings: CRLF becomes LF, LF becomes CRLF.
fn line_ending_twin(sql: &str) -> String {
    if sql.contains("\r\n") {
        sql.replace("\r\n", "\n")
    } else {
        sql.replace('\n', "\r\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// sqlx keys a migration by the number before its first `_`, so two files
    /// on the same date are one version, and no database can apply both (#313).
    #[test]
    fn every_migration_has_a_version_of_its_own() {
        for (set, migrator) in [
            ("migrations", sqlx::migrate!("./migrations")),
            ("migrations/post-content", sqlx::migrate!("./migrations/post-content")),
        ] {
            let mut seen = HashMap::new();
            for m in migrator.iter() {
                if let Some(other) = seen.insert(m.version, &m.description) {
                    panic!("{set}: {} and {} are both version {}", other, m.description, m.version);
                }
            }
        }
    }

    #[test]
    fn twin_flips_line_endings_both_ways() {
        assert_eq!(line_ending_twin("a;\nb;\n"), "a;\r\nb;\r\n");
        assert_eq!(line_ending_twin("a;\r\nb;\r\n"), "a;\nb;\n");
    }

    fn migration(version: i64, sql: &'static str) -> Migration {
        use sqlx::migrate::MigrationType;
        Migration::new(version, "test".into(), MigrationType::Simple, sql.into(), false)
    }

    async fn post_content_versions(db: &PgPool) -> Vec<i64> {
        sqlx::query_scalar(&format!("SELECT version FROM {POST_CONTENT_TABLE} ORDER BY version"))
            .fetch_all(db)
            .await
            .unwrap()
    }

    /// Each runs once, in order, and a failing one takes its whole run back.
    #[sqlx::test(migrations = false)]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn post_content_migrations_run_once_and_all_or_nothing(db: PgPool) {
        let first = migration(1, "CREATE TABLE t (n INT); INSERT INTO t VALUES (1);");
        assert_eq!(apply_post_content(&db, std::slice::from_ref(&first)).await.unwrap(), 1);
        assert_eq!(apply_post_content(&db, std::slice::from_ref(&first)).await.unwrap(), 0);

        let broken = [first, migration(2, "INSERT INTO t VALUES (2);"), migration(3, "INSERT INTO nowhere VALUES (3);")];
        assert!(apply_post_content(&db, &broken).await.is_err());
        assert_eq!(post_content_versions(&db).await, [1]);
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM t").fetch_one(&db).await.unwrap();
        assert_eq!(n, 1, "the failed run's second migration was taken back");

        assert_eq!(apply_post_content(&db, &broken[..2]).await.unwrap(), 1);
        assert_eq!(post_content_versions(&db).await, [1, 2]);
        assert!(apply_post_content(&db, &broken[..1]).await.is_err(), "2 is applied but missing");
    }

    /// An applied migration may change its line endings, but not its SQL.
    #[sqlx::test(migrations = false)]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn an_applied_post_content_migration_keeps_its_sql(db: PgPool) {
        apply_post_content(&db, &[migration(1, "CREATE TABLE t (n INT);\n")]).await.unwrap();
        assert_eq!(apply_post_content(&db, &[migration(1, "CREATE TABLE t (n INT);\r\n")]).await.unwrap(), 0);
        assert!(apply_post_content(&db, &[migration(1, "CREATE TABLE t (n BIGINT);\r\n")]).await.is_err());
    }

    /// The real set on a fresh database, after the content sync, as startup
    /// runs it: it applies, and a second run applies nothing. A later
    /// migration that changes the schema a post-content one relies on fails
    /// here first.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_post_content_migrations_apply_after_the_content_sync(db: PgPool) {
        let seeds: i64 = sqlx::query_scalar("SELECT count(*) FROM resource_node_seeds").fetch_one(&db).await.unwrap();
        let nodes = || sqlx::query_scalar::<_, i64>("SELECT count(*) FROM resource_nodes").fetch_one(&db);
        let before = nodes().await.unwrap();
        assert!(run_post_content(&db).await.is_err(), "the seeds' types don't exist before the sync");

        crate::content::load_and_sync(&db).await.unwrap();
        assert!(run_post_content(&db).await.unwrap() > 0);
        // Every staged seed is planted, then RW1's re-plan (#308) replaces
        // every planned node with its own rows. The first set's nodes (the
        // seeded grass) stay.
        assert!(seeds > 0);
        let replan = include_str!("../migrations/post-content/20261122_001_rural_widens_nodes.sql")
            .lines()
            .filter(|l| l.trim_start().starts_with("('"))
            .count() as i64;
        assert_eq!(nodes().await.unwrap(), before + replan, "the re-plan's nodes and the grass");
        let staging: Option<String> =
            sqlx::query_scalar("SELECT to_regclass('resource_node_seeds')::text").fetch_one(&db).await.unwrap();
        assert_eq!(staging, None);
        assert_eq!(run_post_content(&db).await.unwrap(), 0);
    }
}
