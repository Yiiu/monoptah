//! Context-window accounting, matching sessions/model/contextUsage.ts.
mod restore;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub used: u64,
    pub window: Option<u64>,
}
impl Usage {
    pub fn ratio(self) -> Option<f32> {
        self.window
            .filter(|w| *w > 0)
            .map(|w| (self.used as f64 / w as f64).min(1.) as f32)
    }
    pub fn merge(&mut self, used: Option<u64>, window: Option<u64>) {
        if let Some(used) = used {
            self.used = used;
        }
        if let Some(window) = window.filter(|w| *w > 0) {
            self.window = Some(window);
        }
    }
}

const CLAUDE_TOKEN_FIELDS: [&str; 4] = [
    "input_tokens",
    "cache_creation_input_tokens",
    "cache_read_input_tokens",
    "output_tokens",
];

#[derive(Default)]
pub(crate) struct ClaudeMessageUsage {
    message_id: Option<String>,
    counts: [u64; CLAUDE_TOKEN_FIELDS.len()],
    has_iteration_usage: bool,
}

impl ClaudeMessageUsage {
    pub(crate) fn select_message(&mut self, id: &str) {
        if self.message_id.as_deref().is_some_and(|old| old != id) {
            *self = Self::default();
        }
        self.message_id.get_or_insert_with(|| id.into());
    }

    /// Each frame can report just some of the cumulative counters for one request.
    pub(crate) fn observe(&mut self, usage: &Value) -> Option<u64> {
        if let Some(used) = usage["iterations"]
            .as_array()
            .and_then(|v| v.last())
            .and_then(claude_used)
        {
            self.has_iteration_usage = true;
            return Some(used);
        }
        // After a server-side iteration breakdown, top-level totals span iterations.
        if self.has_iteration_usage {
            return None;
        }
        let mut reported = false;
        for (count, field) in self.counts.iter_mut().zip(CLAUDE_TOKEN_FIELDS) {
            if let Some(value) = usage[field].as_u64() {
                *count = (*count).max(value);
                reported = true;
            }
        }
        let used = self.counts.iter().sum();
        (reported && used > 0).then_some(used)
    }
}

pub fn claude_used(usage: &Value) -> Option<u64> {
    let used = CLAUDE_TOKEN_FIELDS
        .iter()
        .filter_map(|field| usage[field].as_u64())
        .sum();
    (used > 0).then_some(used)
}

pub fn format_tokens(count: u64) -> String {
    let (value, suffix) = if count >= 1_000_000 {
        (count as f64 / 1_000_000., "M")
    } else if count >= 1000 {
        (count as f64 / 1000., "K")
    } else {
        return count.to_string();
    };
    if value < 10. {
        format!(
            "{}{suffix}",
            format!("{value:.1}")
                .trim_end_matches('0')
                .trim_end_matches('.')
        )
    } else {
        format!("{value:.0}{suffix}")
    }
}
