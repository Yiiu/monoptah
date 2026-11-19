//! Context-window accounting, matching sessions/model/contextUsage.ts.
mod restore;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub used: u64,
    pub window: Option<u64>,
    /// Set when a provider supplies the current occupancy. Older saved counts
    /// can instead be totals for several requests in one turn.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub current: bool,
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
            self.current = true;
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
    iteration_count: usize,
}

impl ClaudeMessageUsage {
    pub(crate) fn start_message(&mut self, id: Option<&str>) {
        if id.is_none() || self.message_id.as_deref() != id {
            *self = Self {
                message_id: id.map(str::to_owned),
                ..Self::default()
            };
        }
    }

    pub(crate) fn select_message(&mut self, id: &str) {
        if self.message_id.as_deref().is_some_and(|old| old != id) {
            *self = Self::default();
        }
        self.message_id.get_or_insert_with(|| id.into());
    }

    /// Each frame can report just some of the cumulative counters for one request.
    pub(crate) fn observe(&mut self, usage: &Value) -> Option<u64> {
        let iterations = usage["iterations"].as_array();
        let usage = match iterations.and_then(|list| list.last().map(|last| (list.len(), last))) {
            Some((count, last)) if claude_used(last).is_some() && count >= self.iteration_count => {
                if count > self.iteration_count {
                    self.counts = [0; CLAUDE_TOKEN_FIELDS.len()];
                    self.iteration_count = count;
                }
                last
            }
            // Once an iteration is known, an older snapshot or a turn total
            // cannot refine its per-request counters.
            _ if self.iteration_count > 0 => return None,
            _ => usage,
        };
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
