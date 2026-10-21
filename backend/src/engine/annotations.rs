//! Annotations (the Observatory's D0.4, #354): the things that explain a bend
//! in a chart, kept in `admin_events`.
//!
//! [`record`] is the one place they're written. The server writes one at boot,
//! at each terrain commit and with each Gazette. A tunable changing (D6.1),
//! the town coming and going (D4.1) and a watch firing (D2.5) will write
//! theirs here too. [`note`] is [`record`] for the places an annotation is
//! only a side note: it logs a failure instead of failing what it annotates.
//! [`since`] reads them back, for `/admin/series` and the stream (D0.3).

use capitol_protocol::annotation::{Annotation, AnnotationKind};
use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, PgPool};

/// Write an annotation, now. Returns it as stored.
pub async fn record<'e>(
    db: impl PgExecutor<'e>,
    kind: AnnotationKind,
    text: &str,
) -> Result<Annotation, sqlx::Error> {
    let (id, at): (i64, DateTime<Utc>) =
        sqlx::query_as("INSERT INTO admin_events (kind, text) VALUES ($1, $2) RETURNING id, at")
            .bind(kind.as_str())
            .bind(text)
            .fetch_one(db)
            .await?;
    Ok(Annotation { id, at_ms: at.timestamp_millis(), kind, text: text.to_string() })
}

/// [`record`], logging a failure rather than returning it: a boot, a commit or
/// an edition goes ahead without its marker.
pub async fn note(db: &PgPool, kind: AnnotationKind, text: &str) {
    if let Err(e) = record(db, kind, text).await {
        tracing::error!("Couldn't record the {} annotation \"{text}\": {e}", kind.as_str());
    }
}

/// The annotations since `since_ms` (Unix ms, inclusive), oldest first, at
/// most `limit` of them (the oldest that many).
pub async fn since(db: &PgPool, since_ms: i64, limit: i64) -> Result<Vec<Annotation>, sqlx::Error> {
    let since = DateTime::<Utc>::from_timestamp_millis(since_ms).unwrap_or(DateTime::<Utc>::MIN_UTC);
    let rows: Vec<(i64, DateTime<Utc>, String, String)> = sqlx::query_as(
        "SELECT id, at, kind, text FROM admin_events WHERE at >= $1 ORDER BY at, id LIMIT $2",
    )
    .bind(since)
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, at, kind, text)| {
            // The table's check admits only known kinds
            let kind = AnnotationKind::parse(&kind)?;
            Some(Annotation { id, at_ms: at.timestamp_millis(), kind, text })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    mod db {
        use super::*;

        /// Each kind writes one row and reads back as written, in order, and
        /// `since` leaves out what came before it
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn annotations_are_written_and_read_back(db: PgPool) {
            let mut written = Vec::new();
            for kind in AnnotationKind::ALL {
                written.push(record(&db, kind, &format!("a {}", kind.as_str())).await.unwrap());
            }
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_events").fetch_one(&db).await.unwrap();
            assert_eq!(count, AnnotationKind::ALL.len() as i64);

            let read = since(&db, 0, 100).await.unwrap();
            assert_eq!(read, written);
            assert_eq!(read[0].text, "a boot");

            let last = written.last().unwrap();
            let later = since(&db, last.at_ms + 1, 100).await.unwrap();
            assert!(later.is_empty(), "{later:?}");
            assert_eq!(since(&db, 0, 2).await.unwrap(), written[..2]);
        }

        /// A kind the table doesn't know is refused
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn the_table_refuses_an_unknown_kind(db: PgPool) {
            let refused = sqlx::query("INSERT INTO admin_events (kind, text) VALUES ('nonsense', 'x')")
                .execute(&db)
                .await;
            assert!(refused.is_err());
        }
    }
}
