//! Numeric measurements of the existing row-save path. No payloads are retained.
use std::{cell::RefCell, time::Duration};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveReason {
    #[default]
    Routine,
    Boundary,
    Acceptance,
    Final,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SaveSample {
    pub reason: SaveReason,
    pub history_bytes_read: u64,
    pub serialized_bytes: u64,
    pub changed_rows: usize,
    pub commit_count: u64,
    pub lock_wait: Duration,
    pub lock_held: Duration,
    pub projection: Duration,
    pub encoding: Duration,
    pub commit: Duration,
    pub succeeded: bool,
    /// Chunked-backend write counters, straight from the sparse writer's
    /// flush. Zero until a save actually writes: the blob backend counts
    /// nothing here (its probe layer derives its own `blocks_written`), and a
    /// same-value or failed native save genuinely writes none.
    pub blocks_written: u64,
    pub chunks_written: u64,
    pub chunk_bytes_written: u64,
}

/// Connection-local settings used to pair synthetic measurements. Reading these
/// does not checkpoint or install a WAL hook.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SaveConfiguration {
    pub sqlite_version: String,
    pub page_size: i64,
    pub journal_mode: String,
    pub synchronous: i64,
    pub wal_autocheckpoint: i64,
    pub foreign_keys: i64,
    pub busy_timeout_ms: i64,
    pub cache_size: i64,
    pub temp_store: i64,
    pub locking_mode: String,
    pub compile_options_sha256: String,
}

impl super::Store {
    #[cfg(any(test, feature = "test-support"))]
    pub fn fail_next_save_commit_for_test(&self) -> Result<(), String> {
        let mut first = true;
        self.conn
            .commit_hook(Some(move || std::mem::take(&mut first)))
            .map_err(|error| error.to_string())
    }

    pub fn save_configuration(&self) -> Result<SaveConfiguration, String> {
        use sha2::{Digest, Sha256};
        let integer = |name: &str| {
            self.conn
                .query_row(&format!("PRAGMA {name}"), [], |row| row.get::<_, i64>(0))
                .map_err(|error| error.to_string())
        };
        let text = |name: &str| {
            self.conn
                .query_row(&format!("PRAGMA {name}"), [], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())
        };
        let mut statement = self
            .conn
            .prepare("PRAGMA compile_options")
            .map_err(|error| error.to_string())?;
        let mut options = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        options.sort_unstable();
        Ok(SaveConfiguration {
            sqlite_version: rusqlite::version().into(),
            page_size: integer("page_size")?,
            journal_mode: text("journal_mode")?,
            synchronous: integer("synchronous")?,
            wal_autocheckpoint: integer("wal_autocheckpoint")?,
            foreign_keys: integer("foreign_keys")?,
            busy_timeout_ms: integer("busy_timeout")?,
            cache_size: integer("cache_size")?,
            temp_store: integer("temp_store")?,
            locking_mode: text("locking_mode")?,
            compile_options_sha256: format!("{:x}", Sha256::digest(options.join("\n"))),
        })
    }
}

#[derive(Default)]
pub(super) struct SaveStats {
    // `active` is separate so reads after a save cannot change its untaken sample.
    active: std::cell::Cell<bool>,
    sample: RefCell<Option<SaveSample>>,
}

impl SaveStats {
    pub(super) fn begin(&self, reason: SaveReason) {
        *self.sample.borrow_mut() = Some(SaveSample {
            reason,
            ..Default::default()
        });
        self.active.set(true);
    }

    pub(super) fn update(&self, update: impl FnOnce(&mut SaveSample)) {
        if !self.active.get() {
            return;
        }
        if let Some(sample) = self.sample.borrow_mut().as_mut() {
            update(sample);
        }
    }

    pub(super) fn finish(&self, succeeded: bool) {
        self.update(|sample| sample.succeeded = succeeded);
        self.active.set(false);
    }

    pub(super) fn take(&self) -> Option<SaveSample> {
        self.sample.borrow_mut().take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_lengths_are_bytes() {
        let stats = SaveStats::default();
        stats.begin(SaveReason::Routine);
        let text = "中文🙂";
        assert_eq!(text.len(), 10);
        stats.update(|sample| sample.history_bytes_read += text.len() as u64);
        stats.finish(true);
        stats.update(|sample| sample.history_bytes_read += 100);
        assert_eq!(stats.take().unwrap().history_bytes_read, 10);
        assert!(stats.take().is_none());
    }

    #[test]
    fn stats_do_not_serialize_payloads() {
        let value = serde_json::to_value(SaveSample::default()).unwrap();
        let mut keys = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "blocks_written",
                "changed_rows",
                "chunk_bytes_written",
                "chunks_written",
                "commit",
                "commit_count",
                "encoding",
                "history_bytes_read",
                "lock_held",
                "lock_wait",
                "projection",
                "reason",
                "serialized_bytes",
                "succeeded"
            ]
        );
        for (key, value) in value.as_object().unwrap() {
            if key == "reason" {
                assert_eq!(value, "routine");
            } else {
                assert!(!value.is_string() && !value.is_array());
            }
        }
    }
}
