//! Slow work (D3.2): the database statements that take too long, counted by
//! the phase of the tick, or the loop, they ran in; and, in debug builds, a
//! warning when a phase of the tick awaits the database at all, the class of
//! #304.
//!
//! sqlx logs each statement as an event on the target `sqlx::query`: at
//! debug, or at warn with a `slow_threshold` field once it takes
//! [`statement_logging`]'s threshold or longer. [`SlowWork`] is a layer that
//! sees those events and finds the `tick.<phase>` or `loop.<name>` span they
//! came from. Its filter, [`interest`], is its own, so `RUST_LOG` can't turn
//! the counting off: it takes the engine's spans as well as sqlx's events,
//! since a span no layer wants is never made, and then a statement in it
//! looks as if it ran nowhere.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use sqlx::postgres::PgConnectOptions;
use sqlx::ConnectOptions;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::filter::FilterFn;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::{LookupSpan, Scope};

use super::load::{self, Loop, Phase, Work};

/// sqlx's target for the statements it logs
const TARGET: &str = "sqlx::query";

/// `options` with statements that take `slow` or longer logged as warnings,
/// and the ledger told what slow is.
pub fn statement_logging(options: PgConnectOptions, slow: Duration) -> PgConnectOptions {
    load::slow_threshold(slow);
    options.log_slow_statements(log::LevelFilter::Warn, slow)
}

/// What [`SlowWork`] needs to see: the engine's spans, where the phases and
/// loops are; the slow statements; and in debug builds every statement, to
/// warn of the tick awaiting them.
pub fn interest() -> FilterFn<fn(&Metadata<'_>) -> bool> {
    FilterFn::new(|meta| {
        if meta.is_span() {
            return meta.target().starts_with(ENGINE);
        }
        let level = if cfg!(debug_assertions) { Level::DEBUG } else { Level::WARN };
        meta.target() == TARGET && *meta.level() <= level
    })
}

/// The engine's module path, whose spans hold the phases and loops
const ENGINE: &str = "capitol_backend::engine";

/// Counts slow statements by where they ran, and in debug builds notes the
/// tick's phases that await the database.
pub struct SlowWork;

impl<S> Layer<S> for SlowWork
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        if event.metadata().target() != TARGET {
            return;
        }
        let work = ctx.event_scope(event).map_or(Work::Elsewhere, work_of);
        if event.metadata().fields().field("slow_threshold").is_some() {
            load::slow_statement(work);
        }
        if let Work::Phase(phase) = work {
            if cfg!(debug_assertions) {
                awaited(phase, event);
            }
        }
    }
}

/// The innermost phase or loop in `scope`.
fn work_of<S>(scope: Scope<'_, S>) -> Work
where
    S: for<'a> LookupSpan<'a>,
{
    for span in scope {
        if let Some(phase) = Phase::of_span(span.name()) {
            return Work::Phase(phase);
        }
        if let Some(l) = Loop::of_span(span.name()) {
            return Work::Loop(l);
        }
    }
    Work::Elsewhere
}

/// Statements each phase of the tick has awaited, in debug builds
static AWAITED: [AtomicU64; Phase::ALL.len()] = [const { AtomicU64::new(0) }; Phase::ALL.len()];

/// Awaits to warn of, waiting for [`warn_awaits`]. A layer can't warn
/// itself: tracing drops an event raised while it's handling one.
static TO_WARN: Mutex<Vec<Awaited>> = Mutex::new(Vec::new());

/// A phase of the tick awaited the database, for the `count`th time.
#[derive(Debug, Clone, PartialEq)]
pub struct Awaited {
    pub phase: Phase,
    pub count: u64,
    /// The statement's first words, as sqlx sums it up
    pub statement: String,
}

/// Notes that `phase` awaited a statement, to warn the 1st time, the 10th,
/// the 100th, and so on: enough to see, not so much it drowns the log.
fn awaited(phase: Phase, event: &Event<'_>) {
    let count = AWAITED[Phase::ALL.iter().position(|p| *p == phase).expect("every phase is in ALL")]
        .fetch_add(1, Ordering::Relaxed)
        + 1;
    if !is_power_of_ten(count) {
        return;
    }
    let mut summary = Summary(String::new());
    event.record(&mut summary);
    if let Ok(mut to_warn) = TO_WARN.lock() {
        to_warn.push(Awaited { phase, count, statement: summary.0 });
    }
}

fn is_power_of_ten(mut n: u64) -> bool {
    while n >= 10 && n.is_multiple_of(10) {
        n /= 10;
    }
    n == 1
}

/// The awaits noted since the last call.
pub fn take_awaits() -> Vec<Awaited> {
    TO_WARN.lock().map(|mut w| std::mem::take(&mut *w)).unwrap_or_default()
}

/// Warns of the awaits noted since the last call. The tick calls it once a
/// tick, in debug builds; there are none in release.
pub fn warn_awaits() {
    for a in take_awaits() {
        tracing::warn!(
            "Tick phase {} awaited the database ({} statements so far; this one: {}). The tick waits on it (#304)",
            a.phase.as_str(),
            a.count,
            a.statement,
        );
    }
}

/// sqlx's `summary` field: the statement's first four words.
struct Summary(String);

impl Visit for Summary {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "summary" {
            self.0 = value.to_string();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "summary" {
            self.0 = format!("{value:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::Arc;

    use sqlx::postgres::PgPoolOptions;
    use tracing::Instrument;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::EnvFilter;

    #[test]
    fn it_warns_at_each_power_of_ten() {
        let warned: Vec<u64> = (1..=1000).filter(|n| is_power_of_ten(*n)).collect();
        assert_eq!(warned, [1, 10, 100, 1000]);
        assert!(!is_power_of_ten(0) && !is_power_of_ten(20) && !is_power_of_ten(101));
    }

    #[test]
    fn spans_name_their_phase_or_loop() {
        assert_eq!(Phase::of_span("tick.writes"), Some(Phase::Writes));
        assert_eq!(Loop::of_span("loop.market"), Some(Loop::Market));
        assert_eq!(Loop::of_span("loop.writer"), Some(Loop::Writer));
        assert_eq!(Phase::of_span("loop.market"), None);
        assert_eq!(Phase::of_span("tick.nothing"), None);
        // The phases' and loops' spans are made in the engine
        assert!(module_path!().starts_with(ENGINE));
    }

    /// The log, as the server's console would have shown it.
    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Log {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// D3.2's Verify: a deliberate `pg_sleep` in a phase is logged and
    /// counted. The counts are the process's, so it only looks at what it
    /// changed.
    #[sqlx::test(migrations = false)]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_slow_statement_in_a_phase_is_logged_and_counted(pool: PgPoolOptions, connect: PgConnectOptions) {
        let log = Log::default();
        let writer = log.clone();
        let subscriber = tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(move || writer.clone())
                    // As the server's default, but for its debug lines
                    .with_filter(EnvFilter::new("capitol_backend=info,sqlx::query=warn")),
            )
            .with(SlowWork.with_filter(interest()));
        let _default = tracing::subscriber::set_default(subscriber);
        let db = pool.connect_with(statement_logging(connect, Duration::from_millis(20))).await.unwrap();
        take_awaits();

        let before = load::read(Duration::from_millis(100));
        async { sqlx::query("SELECT pg_sleep(0.05)").execute(&db).await.unwrap() }
            .instrument(tracing::info_span!("tick.tax"))
            .await;
        async { sqlx::query("SELECT pg_sleep(0.05)").execute(&db).await.unwrap() }
            .instrument(tracing::info_span!("loop.bazaar"))
            .await;
        sqlx::query("SELECT pg_sleep(0.05)").execute(&db).await.unwrap();
        // Quick, in a phase that has no business awaiting anything
        async { sqlx::query("SELECT 1").execute(&db).await.unwrap() }
            .instrument(tracing::info_span!("tick.movement"))
            .await;
        let after = load::read(Duration::from_millis(100));

        let slow = |l: &capitol_protocol::ledger::Load, phase: &str| l.phases[phase].slow_statements;
        assert_eq!(slow(&after, "tax") - slow(&before, "tax"), 1);
        assert_eq!(slow(&after, "movement"), slow(&before, "movement"), "SELECT 1 isn't slow");
        assert_eq!(after.loops["bazaar"].slow_statements - before.loops["bazaar"].slow_statements, 1);
        assert!(after.slow_statements_elsewhere > before.slow_statements_elsewhere);
        assert_eq!(after.slow_statement_ms, 20.0);

        let log = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
        assert_eq!(log.matches("slow statement").count(), 3, "{log}");
        assert!(log.contains("tick.tax"), "the phase is in the line: {log}");

        if cfg!(debug_assertions) {
            let awaits = take_awaits();
            let phases: Vec<Phase> = awaits.iter().map(|a| a.phase).collect();
            assert!(phases.contains(&Phase::Movement), "{awaits:?}");
            let movement = awaits.iter().find(|a| a.phase == Phase::Movement).unwrap();
            assert_eq!(movement.statement, "SELECT 1");
        }
    }
}
