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

use sha2::{Digest, Sha384};
use sqlx::PgPool;

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
            let twin = Sha384::digest(line_ending_twin(&m.sql).as_bytes());
            if stored.as_slice() == twin.as_slice() {
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

    #[test]
    fn twin_flips_line_endings_both_ways() {
        assert_eq!(line_ending_twin("a;\nb;\n"), "a;\r\nb;\r\n");
        assert_eq!(line_ending_twin("a;\r\nb;\r\n"), "a;\nb;\n");
    }
}
