//! The Observatory's watches (D2.5, #364): rules that warn, run by the
//! sampler.
//!
//! The rules are `capitol_protocol::watch`'s. [`Watcher`] feeds them each
//! sample: the books between the sample's two ledgers, the units since the
//! last ledger read in a quiet moment, the balances of people and citizens,
//! the deeds held and the sample itself. The thresholds are the live
//! tunables (`Tunables::watches`), read each sample, so a change to one takes
//! on the next. A watch that fires is written as a `watch_fired` annotation,
//! which goes out on the admin stream, and each sample lists the watches
//! firing as it was taken.
//!
//! No item has a target price yet: D2.3 (#362) puts `target_price` in the
//! content, and until then the off-target watch has nothing to watch.

use std::collections::{BTreeMap, HashSet};

use capitol_protocol::annotation::AnnotationKind;
use capitol_protocol::ledger::{reconcile, Ledger};
use capitol_protocol::metrics::Sample;
use capitol_protocol::watch::{Reading, Watches};
use sqlx::PgPool;
use uuid::Uuid;

use super::annotations;
use crate::state::AppState;

/// The watches, and the last ledger whose units were read in a quiet moment.
#[derive(Default)]
pub struct Watcher {
    watches: Watches,
    settled: Option<Ledger>,
}

impl Watcher {
    pub fn new() -> Watcher {
        Watcher::default()
    }

    /// Judge `sample`, taken between the ledgers `last` and `now`, and say
    /// in it which watches are firing. Each that fired with it is annotated.
    pub async fn check(&mut self, state: &AppState, last: &Ledger, now: &Ledger, admins: &HashSet<Uuid>, sample: &mut Sample) {
        let deeds = match deeds(&state.db).await {
            Ok(deeds) => deeds,
            Err(e) => {
                // Judged next sample; the watches keep what they had
                tracing::warn!("The watches couldn't count the deeds: {e}");
                sample.watches = self.watches.firing();
                sample.drift = self.watches.drift();
                return;
            }
        };
        let fired = {
            let reading = self.reading(last, now, admins, sample, deeds);
            // D2.3 (#362) gives items their target prices
            let targets = BTreeMap::new();
            self.watches.observe(&reading, &state.tunables.get().watches, &targets)
        };
        for fired in fired {
            tracing::warn!("Watch: {}", fired.text);
            annotations::note(&state.db, AnnotationKind::WatchFired, &fired.text).await;
        }
        sample.watches = self.watches.firing();
        sample.drift = self.watches.drift();
    }

    fn reading(&mut self, last: &Ledger, now: &Ledger, admins: &HashSet<Uuid>, sample: &Sample, deeds: u64) -> Reading {
        let strands = reconcile(last, now).strands;
        // Units only agree with their flows in a quiet moment, so they're
        // judged from one such ledger to the next. In a busy town that's
        // seldom (#408).
        let units = if now.units_settled {
            let units = self.settled.as_ref().map(|settled| reconcile(settled, now).units);
            self.settled = Some(now.clone());
            units
        } else {
            None
        };
        let balances =
            now.accounts.iter().filter(|a| !admins.contains(&a.player_id)).map(|a| a.strands).collect();
        Reading::of(sample, &strands, units.as_ref(), balances, deeds)
    }
}

/// The deeds held now.
async fn deeds(db: &PgPool) -> Result<u64, sqlx::Error> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plots WHERE owner_id IS NOT NULL").fetch_one(db).await?;
    Ok(n as u64)
}
