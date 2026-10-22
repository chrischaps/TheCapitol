//! Letters (commons plan D4, S1.5): words that wait for someone who's away.
//!
//! A letter costs [`talk::POSTAGE`], a sink, and goes in one transaction
//! with its postage and its `letter_arrived` notice ([`notices::queue`]), so
//! a reader who's away hears of it when they come back, once. Each side
//! keeps its own copy: the writer's out box and the reader's in box drop
//! theirs apart, and the row goes once neither keeps it.
//!
//! Each box keeps [`talk::MAILBOX`] letters. Past it the oldest read letters
//! go first; only a box of nothing but unread letters loses its oldest
//! unread. A writer's own letters are all read to them, so the out box
//! loses its oldest.
//!
//! A reader who's muted the writer (S1.4) never gets the letter, and the
//! writer can't tell: it's written, paid for and in their out box as any
//! other, and nobody is told whether a letter was read.

use capitol_protocol::rest::{Correspondent, Letter, LetterBox, Mailbox};
use capitol_protocol::rules::talk::{self, TalkError};
use capitol_protocol::ws::ServerMessage;
use chrono::{DateTime, SubsecRound, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::currency::{subtract_strands_in, TransactionType};
use super::notices;

/// How many letters a page of a box holds.
pub const PAGE: usize = 50;

/// Why a letter, or something done with one, was refused.
#[derive(Debug)]
pub enum LetterError {
    Text(TalkError),
    ToYourself,
    /// Nobody by that id
    NoOne,
    /// Not enough Strands for the postage
    Postage,
    /// No such letter in your boxes
    NoLetter,
    Db(sqlx::Error),
}

impl LetterError {
    /// Words a client can show.
    pub fn message(&self) -> String {
        match self {
            LetterError::Text(e) => e.message(),
            LetterError::ToYourself => "You can't write to yourself".into(),
            LetterError::NoOne => "No one goes by that".into(),
            LetterError::Postage => format!(
                "A letter's postage is {} Strand, and you haven't one",
                talk::POSTAGE
            ),
            LetterError::NoLetter => "There's no such letter in your boxes".into(),
            LetterError::Db(e) => format!("Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for LetterError {
    fn from(e: sqlx::Error) -> Self {
        LetterError::Db(e)
    }
}

/// A letter sent, and its notice for the reader, unless they muted the writer.
#[derive(Debug)]
pub struct Sent {
    pub letter: Letter,
    /// The reader, the notice's id and the `letter_arrived` to send live
    pub notice: Option<(Uuid, Uuid, ServerMessage)>,
}

/// `from` writes `body` to `to`, at `now`, paying the postage. `muted` is
/// whether `to` has muted `from` (S1.4): then it's written and paid for, but
/// only into `from`'s out box.
pub async fn send(
    db: &PgPool,
    from: Uuid,
    to: Uuid,
    body: &str,
    muted: bool,
    now: DateTime<Utc>,
) -> Result<Sent, LetterError> {
    let body = talk::clean_letter(body).map_err(LetterError::Text)?;
    // As Postgres keeps it, so the letter sent back reads as the box will
    let now = now.trunc_subsecs(6);
    if from == to {
        return Err(LetterError::ToYourself);
    }
    let mut tx = db.begin().await?;
    let writer = correspondent(&mut tx, from).await?.ok_or(LetterError::NoOne)?;
    let reader = correspondent(&mut tx, to).await?.ok_or(LetterError::NoOne)?;

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO letters (id, sender_id, recipient_id, body, sent_at, kept_by_recipient)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(from)
    .bind(to)
    .bind(&body)
    .bind(now)
    .bind(!muted)
    .execute(&mut *tx)
    .await?;
    subtract_strands_in(
        &mut tx,
        from,
        talk::POSTAGE,
        TransactionType::Postage,
        Some(&format!("Postage to {}", reader.name)),
        Some(id),
    )
    .await
    .map_err(|e| if e.starts_with("Insufficient") { LetterError::Postage } else { LetterError::Db(sqlx::Error::Protocol(e)) })?;

    let notice = if muted {
        None
    } else {
        let message = ServerMessage::LetterArrived {
            id,
            from,
            name: writer.name.clone(),
            citizen: writer.citizen,
            first_line: talk::first_line(&body),
        };
        let notice_id = notices::queue(&mut *tx, to, &id.to_string(), &message).await?;
        trim(&mut tx, to, LetterBox::In).await?;
        Some((to, notice_id, message))
    };
    trim(&mut tx, from, LetterBox::Out).await?;
    tx.commit().await?;

    Ok(Sent {
        letter: Letter { id, from: writer, to: reader, body, sent_at: now, read_at: None },
        notice,
    })
}

/// A page of `player`'s `which` box, newest first: the letters sent before
/// the letter `before`, if it's given.
pub async fn page(db: &PgPool, player: Uuid, which: LetterBox, before: Option<Uuid>) -> Result<Mailbox, LetterError> {
    let (owner, kept) = columns(which);
    type Row = (Uuid, String, DateTime<Utc>, Option<DateTime<Utc>>, Uuid, String, bool, Uuid, String, bool);
    let mut rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT l.id, l.body, l.sent_at, l.read_at,
                s.id, s.name, sa.kind = 'citizen', r.id, r.name, ra.kind = 'citizen'
         FROM letters l
         JOIN players s ON s.id = l.sender_id JOIN accounts sa ON sa.id = s.account_id
         JOIN players r ON r.id = l.recipient_id JOIN accounts ra ON ra.id = r.account_id
         WHERE l.{owner} = $1 AND l.{kept}
           AND ($2::uuid IS NULL OR (l.sent_at, l.id) < (SELECT sent_at, id FROM letters WHERE id = $2))
         ORDER BY l.sent_at DESC, l.id DESC
         LIMIT $3"
    ))
    .bind(player)
    .bind(before)
    .bind(PAGE as i64 + 1)
    .fetch_all(db)
    .await?;
    let more = rows.len() > PAGE;
    rows.truncate(PAGE);
    let letters = rows
        .into_iter()
        .map(|(id, body, sent_at, read_at, s, s_name, s_citizen, r, r_name, r_citizen)| Letter {
            id,
            from: Correspondent { id: s, name: s_name, citizen: s_citizen },
            to: Correspondent { id: r, name: r_name, citizen: r_citizen },
            body,
            sent_at,
            // A writer isn't told whether it was read
            read_at: read_at.filter(|_| which == LetterBox::In),
        })
        .collect();
    Ok(Mailbox { letters, unread: unread(db, player).await?, more })
}

/// `player` opened the letter `id`, at `now`. Opening it again keeps the
/// first time. Returns their in box's unread letters now.
pub async fn read(db: &PgPool, player: Uuid, id: Uuid, now: DateTime<Utc>) -> Result<u32, LetterError> {
    let opened = sqlx::query(
        "UPDATE letters SET read_at = COALESCE(read_at, $3)
         WHERE id = $1 AND recipient_id = $2 AND kept_by_recipient",
    )
    .bind(id)
    .bind(player)
    .bind(now)
    .execute(db)
    .await?;
    if opened.rows_affected() == 0 {
        return Err(LetterError::NoLetter);
    }
    Ok(unread(db, player).await?)
}

/// `player` drops their copy of the letter `id`, from whichever of their
/// boxes it's in. Returns their in box's unread letters now.
pub async fn drop_letter(db: &PgPool, player: Uuid, id: Uuid) -> Result<u32, LetterError> {
    let mut tx = db.begin().await?;
    let mut dropped = 0;
    for which in [LetterBox::In, LetterBox::Out] {
        let (owner, kept) = columns(which);
        let mine: Vec<Uuid> = sqlx::query_scalar(&format!("SELECT id FROM letters WHERE id = $1 AND {owner} = $2 AND {kept}"))
            .bind(id)
            .bind(player)
            .fetch_all(&mut *tx)
            .await?;
        dropped += forget(&mut tx, which, &mine).await?;
    }
    if dropped == 0 {
        return Err(LetterError::NoLetter);
    }
    tx.commit().await?;
    Ok(unread(db, player).await?)
}

/// The letters unread in `player`'s in box.
pub async fn unread(db: &PgPool, player: Uuid) -> Result<u32, sqlx::Error> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM letters WHERE recipient_id = $1 AND kept_by_recipient AND read_at IS NULL",
    )
    .bind(player)
    .fetch_one(db)
    .await?;
    Ok(n as u32)
}

/// Who `player` is on a letter, or `None` if there's no such player.
async fn correspondent(conn: &mut PgConnection, player: Uuid) -> Result<Option<Correspondent>, sqlx::Error> {
    let row: Option<(String, bool)> = sqlx::query_as(
        "SELECT p.name, a.kind = 'citizen' FROM players p JOIN accounts a ON a.id = p.account_id WHERE p.id = $1",
    )
    .bind(player)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|(name, citizen)| Correspondent { id: player, name, citizen }))
}

/// Drops what's past [`talk::MAILBOX`] from `player`'s `which` box: keeps
/// the unread first, then the newest.
async fn trim(conn: &mut PgConnection, player: Uuid, which: LetterBox) -> Result<(), sqlx::Error> {
    let (owner, kept) = columns(which);
    let keep_first = match which {
        LetterBox::In => "(read_at IS NULL) DESC, sent_at DESC, id DESC",
        LetterBox::Out => "sent_at DESC, id DESC",
    };
    let past: Vec<Uuid> = sqlx::query_scalar(&format!(
        "SELECT id FROM letters WHERE {owner} = $1 AND {kept} ORDER BY {keep_first} OFFSET {}",
        talk::MAILBOX
    ))
    .bind(player)
    .fetch_all(&mut *conn)
    .await?;
    forget(conn, which, &past).await?;
    Ok(())
}

/// Drops `which` side's copy of the letters `ids`: a row goes if the other
/// side has dropped theirs. Returns how many were dropped.
async fn forget(conn: &mut PgConnection, which: LetterBox, ids: &[Uuid]) -> Result<u64, sqlx::Error> {
    if ids.is_empty() {
        return Ok(0);
    }
    let (_, kept) = columns(which);
    let (_, other) = columns(match which {
        LetterBox::In => LetterBox::Out,
        LetterBox::Out => LetterBox::In,
    });
    let (gone, dropped): (i64, i64) = sqlx::query_as(&format!(
        "WITH gone AS (DELETE FROM letters WHERE id = ANY($1) AND NOT {other} RETURNING id),
         dropped AS (UPDATE letters SET {kept} = FALSE WHERE id = ANY($1) AND {other} RETURNING id)
         SELECT (SELECT COUNT(*) FROM gone), (SELECT COUNT(*) FROM dropped)"
    ))
    .bind(ids)
    .fetch_one(conn)
    .await?;
    Ok((gone + dropped) as u64)
}

/// The column naming a box's owner, and the one saying they keep it.
fn columns(which: LetterBox) -> (&'static str, &'static str) {
    match which {
        LetterBox::In => ("recipient_id", "kept_by_recipient"),
        LetterBox::Out => ("sender_id", "kept_by_sender"),
    }
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test letters -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::conservation::{assert_audited, balances_of};
    use crate::engine::plot_sync::tests::player;
    use capitol_protocol::ledger::StrandFlow;
    use chrono::Duration;

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000, 0).unwrap() + Duration::minutes(minutes)
    }

    async fn postage_paid(db: &PgPool) -> i64 {
        sqlx::query_scalar("SELECT COALESCE(SUM(amount), 0)::BIGINT FROM currency_transactions WHERE transaction_type = 'postage'")
            .fetch_one(db)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_letter_waits_for_an_offline_reader_and_arrives_once(db: PgPool) {
        let ada = player(&db, "Ada", 10).await;
        let bea = player(&db, "Bea", 10).await;
        let sent = send(&db, ada, bea, "  Dear Bea,\n\n\n\nThe rope held.  ", false, at(0)).await.unwrap();
        assert_eq!(sent.letter.body, "Dear Bea,\n\nThe rope held.", "cleaned, its paragraphs kept");
        assert_eq!((sent.letter.from.name.as_str(), sent.letter.to.name.as_str()), ("Ada", "Bea"));

        // Bea was away: it waits, and comes on her return, once
        let waiting = notices::take_all(&db, bea, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap();
        assert_eq!(
            waiting,
            vec![ServerMessage::LetterArrived {
                id: sent.letter.id,
                from: ada,
                name: "Ada".into(),
                citizen: false,
                first_line: "The rope held.".into(),
            }]
        );
        assert!(notices::take_all(&db, bea, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap().is_empty());
        // Sent live instead, it isn't sent again
        let (_, notice_id, _) = send(&db, ada, bea, "And again.", false, at(1)).await.unwrap().notice.unwrap();
        assert!(notices::take(&db, notice_id).await.unwrap());
        assert!(notices::take_all(&db, bea, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap().is_empty());

        let inbox = page(&db, bea, LetterBox::In, None).await.unwrap();
        assert_eq!((inbox.letters.len(), inbox.unread, inbox.more), (2, 2, false));
        assert_eq!(inbox.letters[0].body, "And again.", "newest first");
        let outbox = page(&db, ada, LetterBox::Out, None).await.unwrap();
        assert_eq!((outbox.letters.len(), outbox.unread), (2, 0));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn postage_is_a_strand_sunk_and_audited(db: PgPool) {
        let ada = player(&db, "Ada", 2).await;
        let bea = player(&db, "Bea", 0).await;
        let start = balances_of(&db, [ada, bea]).await;
        send(&db, ada, bea, "One.", false, at(0)).await.unwrap();
        send(&db, ada, bea, "Two.", false, at(1)).await.unwrap();
        let broke = send(&db, ada, bea, "Three.", false, at(2)).await.unwrap_err();
        assert!(matches!(broke, LetterError::Postage), "{broke:?}");
        assert!(broke.message().contains("postage is 1 Strand"));
        assert_eq!(balances_of(&db, [ada]).await[&ada], 0);
        assert_eq!(postage_paid(&db).await, -2 * talk::POSTAGE);
        assert_audited(&db, &start).await;
        // The refused one left nothing behind
        assert_eq!(page(&db, bea, LetterBox::In, None).await.unwrap().letters.len(), 2);
        assert_eq!(StrandFlow::of(TransactionType::Postage.as_str()), StrandFlow::Sink);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn refusals_cost_nothing(db: PgPool) {
        let ada = player(&db, "Ada", 10).await;
        assert!(matches!(send(&db, ada, ada, "Me", false, at(0)).await, Err(LetterError::ToYourself)));
        assert!(matches!(send(&db, ada, Uuid::new_v4(), "Who?", false, at(0)).await, Err(LetterError::NoOne)));
        assert!(matches!(send(&db, ada, Uuid::new_v4(), " \n ", false, at(0)).await, Err(LetterError::Text(TalkError::Empty))));
        let long = "a".repeat(talk::MAX_LETTER + 1);
        assert!(matches!(send(&db, ada, Uuid::new_v4(), &long, false, at(0)).await, Err(LetterError::Text(TalkError::TooLong { .. }))));
        assert_eq!(postage_paid(&db).await, 0);
    }

    /// A muted writer pays and keeps their copy, as for any letter; the
    /// reader never sees it
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_muted_writers_letter_never_arrives_and_they_cant_tell(db: PgPool) {
        let ada = player(&db, "Ada", 10).await;
        let bea = player(&db, "Bea", 10).await;
        let sent = send(&db, ada, bea, "Hello?", true, at(0)).await.unwrap();
        assert!(sent.notice.is_none());
        assert!(notices::take_all(&db, bea, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap().is_empty());
        assert!(page(&db, bea, LetterBox::In, None).await.unwrap().letters.is_empty());
        let out = page(&db, ada, LetterBox::Out, None).await.unwrap();
        assert_eq!(out.letters.len(), 1);
        assert_eq!(out.letters[0].read_at, None);
        assert_eq!(postage_paid(&db).await, -talk::POSTAGE);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn reading_and_dropping_touch_only_your_copy(db: PgPool) {
        let ada = player(&db, "Ada", 10).await;
        let bea = player(&db, "Bea", 10).await;
        let cy = player(&db, "Cy", 10).await;
        let id = send(&db, ada, bea, "Hello", false, at(0)).await.unwrap().letter.id;

        assert!(matches!(read(&db, ada, id, at(1)).await, Err(LetterError::NoLetter)), "the writer doesn't read it");
        assert_eq!(read(&db, bea, id, at(5)).await.unwrap(), 0);
        assert_eq!(read(&db, bea, id, at(9)).await.unwrap(), 0);
        let inbox = page(&db, bea, LetterBox::In, None).await.unwrap();
        assert_eq!(inbox.letters[0].read_at, Some(at(5)), "the first time it was opened");
        assert_eq!(page(&db, ada, LetterBox::Out, None).await.unwrap().letters[0].read_at, None, "nobody's told");

        assert!(matches!(drop_letter(&db, cy, id).await, Err(LetterError::NoLetter)));
        drop_letter(&db, bea, id).await.unwrap();
        assert!(page(&db, bea, LetterBox::In, None).await.unwrap().letters.is_empty());
        assert_eq!(page(&db, ada, LetterBox::Out, None).await.unwrap().letters.len(), 1, "Ada keeps hers");
        assert!(matches!(drop_letter(&db, bea, id).await, Err(LetterError::NoLetter)), "dropped once");
        drop_letter(&db, ada, id).await.unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM letters").fetch_one(&db).await.unwrap();
        assert_eq!(rows, 0, "neither keeps it, so it's gone");
    }

    /// Past 200, the oldest read go first, then the oldest unread; the out
    /// box loses its oldest
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_full_box_drops_the_oldest_read_letters_first(db: PgPool) {
        let ada = player(&db, "Ada", 1000).await;
        let bea = player(&db, "Bea", 10).await;
        let mut ids = Vec::new();
        for n in 0..talk::MAILBOX as i64 {
            ids.push(send(&db, ada, bea, &format!("Letter {n}"), false, at(n)).await.unwrap().letter.id);
        }
        // Bea has read the 10th and 20th, not the oldest
        read(&db, bea, ids[10], at(500)).await.unwrap();
        read(&db, bea, ids[20], at(501)).await.unwrap();

        send(&db, ada, bea, "Letter 200", false, at(600)).await.unwrap();
        let kept = all(&db, bea, LetterBox::In).await;
        assert_eq!(kept.len(), talk::MAILBOX);
        assert!(!kept.contains(&ids[10]) && kept.contains(&ids[20]) && kept.contains(&ids[0]), "the oldest read went");

        send(&db, ada, bea, "Letter 201", false, at(601)).await.unwrap();
        send(&db, ada, bea, "Letter 202", false, at(602)).await.unwrap();
        let kept = all(&db, bea, LetterBox::In).await;
        assert!(!kept.contains(&ids[20]), "the last read one");
        assert!(!kept.contains(&ids[0]) && kept.contains(&ids[1]), "then the oldest unread");

        let out = all(&db, ada, LetterBox::Out).await;
        assert_eq!(out.len(), talk::MAILBOX);
        assert!(!out.contains(&ids[2]) && out.contains(&ids[3]), "the out box loses its oldest");
        // Neither keeps the 1st now, so its row is gone; Bea keeps the 2nd
        // and 3rd, which Ada dropped, and Ada the 11th, which Bea dropped
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM letters WHERE id = ANY($1)")
            .bind(&ids[..11])
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(rows, 10, "all but the 1st");
    }

    /// Every id in `player`'s `which` box, page by page
    async fn all(db: &PgPool, player: Uuid, which: LetterBox) -> Vec<Uuid> {
        let mut ids = Vec::new();
        let mut before = None;
        loop {
            let box_ = page(db, player, which, before).await.unwrap();
            assert!(box_.letters.len() <= PAGE);
            ids.extend(box_.letters.iter().map(|l| l.id));
            before = ids.last().copied();
            if !box_.more {
                return ids;
            }
        }
    }
}
