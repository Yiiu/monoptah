//! Opt-in, bounded numeric summaries. A save never writes its own log record.
use gpui::App;
use monocode_core::store::{SaveReason, SaveSample};
use std::{
    sync::{Mutex, OnceLock},
    time::Duration,
};

pub(crate) const TARGET: &str = "persistence_stats";
const PERIOD: Duration = Duration::from_secs(60);
static ENABLED: OnceLock<bool> = OnceLock::new();
static TOTALS: Mutex<Totals> = Mutex::new(Totals::new());

#[derive(Default, serde::Serialize)]
struct Totals {
    samples: u64,
    failures: u64,
    reasons: [u64; 4],
    // history bytes, encoded bytes, changed rows, commits, lock wait ns,
    // lock held ns, projection ns, encoding ns, commit ns.
    sum: [u64; 9],
    max: [u64; 9],
}

impl Totals {
    const fn new() -> Self {
        Self {
            samples: 0,
            failures: 0,
            reasons: [0; 4],
            sum: [0; 9],
            max: [0; 9],
        }
    }

    fn add(&mut self, sample: SaveSample) {
        self.samples = self.samples.saturating_add(1);
        self.failures = self.failures.saturating_add(u64::from(!sample.succeeded));
        let reason = match sample.reason {
            SaveReason::Routine => 0,
            SaveReason::Boundary => 1,
            SaveReason::Acceptance => 2,
            SaveReason::Final => 3,
        };
        self.reasons[reason] = self.reasons[reason].saturating_add(1);
        let ns = |duration: Duration| duration.as_nanos().min(u64::MAX as u128) as u64;
        let values = [
            sample.history_bytes_read,
            sample.serialized_bytes,
            sample.changed_rows as u64,
            sample.commit_count,
            ns(sample.lock_wait),
            ns(sample.lock_held),
            ns(sample.projection),
            ns(sample.encoding),
            ns(sample.commit),
        ];
        for (index, value) in values.into_iter().enumerate() {
            self.sum[index] = self.sum[index].saturating_add(value);
            self.max[index] = self.max[index].max(value);
        }
    }
}

fn enabled() -> bool {
    *ENABLED
        .get_or_init(|| std::env::var("MONOCODE_PERSISTENCE_STATS").is_ok_and(|value| value == "1"))
}

pub(crate) fn record(sample: SaveSample) {
    if enabled()
        && let Ok(mut totals) = TOTALS.lock()
    {
        totals.add(sample);
    }
}

pub(crate) fn start(cx: &mut App) {
    if !enabled() {
        return;
    }
    let executor = cx.background_executor().clone();
    cx.background_executor()
        .spawn(async move {
            loop {
                executor.timer(PERIOD).await;
                let totals = TOTALS
                    .lock()
                    .ok()
                    .map(|mut totals| std::mem::take(&mut *totals));
                if let Some(totals) = totals
                    && let Ok(line) = serde_json::to_string(&totals)
                {
                    log::info!(target: TARGET, "{line}");
                }
            }
        })
        .detach();
}

#[cfg(test)]
mod persistence_safety_tests {
    use super::*;

    #[test]
    fn stats_do_not_serialize_payloads() {
        let mut totals = Totals::new();
        totals.add(SaveSample {
            reason: SaveReason::Acceptance,
            serialized_bytes: 10,
            lock_wait: Duration::from_nanos(7),
            succeeded: true,
            ..Default::default()
        });
        let value = serde_json::to_value(totals).unwrap();
        // serde_json emits struct fields in declaration order; the set is the contract.
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["samples", "failures", "reasons", "sum", "max"]
        );
        assert_eq!(value["reasons"], serde_json::json!([0, 0, 1, 0]));
        assert_eq!(value["sum"][1], 10);
        assert_eq!(value["max"][4], 7);
        assert!(value.as_object().unwrap().values().all(|value| {
            value.is_number()
                || value
                    .as_array()
                    .is_some_and(|values| values.iter().all(serde_json::Value::is_u64))
        }));
    }
}
