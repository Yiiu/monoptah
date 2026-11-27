//! Provider-neutral activity shown for one agent session.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Approval,
    Question,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    Subtasks(u32),
    UsageLimit { resume_at: u64 },
    UsageLimitPending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Failed,
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurnEnd {
    pub at: u64,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellTask {
    pub id: String,
    pub command: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Activity {
    NeedsYou {
        kind: Ask,
    },
    Running {
        step: Option<String>,
        since: u64,
        stale: bool,
    },
    BackgroundWait {
        reason: Wait,
        since: u64,
    },
    Done {
        outcome: Outcome,
        at: u64,
        unread: bool,
    },
    #[default]
    Idle,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentStatus {
    pub activity: Activity,
    pub shells: Vec<ShellTask>,
}

pub struct StatusInput {
    pub approvals: bool,
    pub questions: bool,
    pub remote_waiting: bool,
    pub streaming: bool,
    pub wake_tasks: u32,
    pub usage_limit_resume_at: Option<u64>,
    pub usage_limit_pending: bool,
    pub shells: Vec<ShellTask>,
    pub step: Option<String>,
    pub state_since: u64,
    pub last_event_at: u64,
    /// A TUI without an observable terminal grid cannot diagnose silence.
    pub stale_applicable: bool,
    pub last_turn: Option<TurnEnd>,
    pub seen_at: Option<u64>,
    pub now: u64,
}

impl Default for StatusInput {
    fn default() -> Self {
        Self {
            approvals: false,
            questions: false,
            remote_waiting: false,
            streaming: false,
            wake_tasks: 0,
            usage_limit_resume_at: None,
            usage_limit_pending: false,
            shells: Vec::new(),
            step: None,
            state_since: 0,
            last_event_at: 0,
            stale_applicable: true,
            last_turn: None,
            seen_at: None,
            now: 0,
        }
    }
}

const STALE_AFTER_MS: u64 = 30 * 60 * 1000;

pub fn derive(input: &StatusInput) -> AgentStatus {
    let ask = if input.approvals {
        Some(Ask::Approval)
    } else if input.questions {
        Some(Ask::Question)
    } else if input.remote_waiting {
        Some(Ask::Remote)
    } else {
        None
    };
    let activity = if let Some(kind) = ask {
        Activity::NeedsYou { kind }
    } else if input.streaming {
        Activity::Running {
            step: input.step.clone(),
            since: input.state_since,
            stale: input.stale_applicable
                && input.now.saturating_sub(input.last_event_at) >= STALE_AFTER_MS,
        }
    } else if input.wake_tasks > 0 {
        Activity::BackgroundWait {
            reason: Wait::Subtasks(input.wake_tasks),
            since: input.state_since,
        }
    } else if let Some(resume_at) = input.usage_limit_resume_at {
        Activity::BackgroundWait {
            reason: Wait::UsageLimit { resume_at },
            since: input.state_since,
        }
    } else if input.usage_limit_pending {
        Activity::BackgroundWait {
            reason: Wait::UsageLimitPending,
            since: input.state_since,
        }
    } else if let Some(turn) = input.last_turn {
        Activity::Done {
            outcome: turn.outcome,
            at: turn.at,
            unread: input.seen_at.is_none_or(|seen| seen < turn.at),
        }
    } else {
        Activity::Idle
    };
    AgentStatus {
        activity,
        shells: input.shells.clone(),
    }
}

impl Activity {
    pub fn is_animating(&self) -> bool {
        matches!(self, Self::Running { stale: false, .. })
    }

    pub fn label(&self, now: u64) -> Option<String> {
        Some(match self {
            Activity::NeedsYou {
                kind: Ask::Approval,
            } => "Needs approval".into(),
            Activity::NeedsYou { .. } => "Needs input".into(),
            Activity::Running { stale: true, .. } => "No update in 30m".into(),
            Activity::Running { step, since, .. } => {
                let step = step.as_deref().unwrap_or("Working");
                format!("{step} · {}", elapsed(now.saturating_sub(*since)))
            }
            Activity::BackgroundWait {
                reason: Wait::Subtasks(count),
                ..
            } => {
                let noun = if *count == 1 { "subtask" } else { "subtasks" };
                format!("Waiting on {count} {noun}")
            }
            Activity::BackgroundWait {
                reason: Wait::UsageLimit { resume_at },
                ..
            } => {
                let remaining = resume_at.saturating_sub(now);
                if remaining == 0 {
                    "Usage limit · resuming".into()
                } else {
                    format!("Usage limit · resumes in {}", elapsed(remaining))
                }
            }
            Activity::BackgroundWait {
                reason: Wait::UsageLimitPending,
                ..
            } => "Usage limit · waiting for reset".into(),
            Activity::Done {
                outcome,
                unread: true,
                ..
            } => match outcome {
                Outcome::Completed => "Done",
                Outcome::Failed => "Failed",
                Outcome::Interrupted => "Stopped",
            }
            .into(),
            Activity::Done { .. } | Activity::Idle => return None,
        })
    }
}

impl AgentStatus {
    pub fn label(&self, now: u64) -> Option<String> {
        self.activity.label(now)
    }

    pub fn needs_you(&self) -> bool {
        matches!(self.activity, Activity::NeedsYou { .. })
    }

    pub fn is_busy(&self) -> bool {
        matches!(
            self.activity,
            Activity::Running { .. } | Activity::BackgroundWait { .. }
        )
    }

    pub fn unread_done(&self) -> bool {
        matches!(self.activity, Activity::Done { unread: true, .. })
    }
}

fn elapsed(ms: u64) -> String {
    let seconds = ms / 1000;
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m", seconds / 60),
        _ => format!("{}h {}m", seconds / 3600, (seconds / 60) % 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_you_then_running_then_wake_work_then_usage_limit_then_done() {
        let mut input = StatusInput {
            approvals: true,
            questions: true,
            remote_waiting: true,
            streaming: true,
            wake_tasks: 2,
            usage_limit_resume_at: Some(100),
            state_since: 10,
            last_turn: Some(TurnEnd {
                at: 9,
                outcome: Outcome::Failed,
            }),
            ..Default::default()
        };
        assert_eq!(
            derive(&input).activity,
            Activity::NeedsYou {
                kind: Ask::Approval
            }
        );
        input.approvals = false;
        assert_eq!(
            derive(&input).activity,
            Activity::NeedsYou {
                kind: Ask::Question
            }
        );
        input.questions = false;
        assert_eq!(
            derive(&input).activity,
            Activity::NeedsYou { kind: Ask::Remote }
        );
        input.remote_waiting = false;
        assert_eq!(
            derive(&input).activity,
            Activity::Running {
                step: None,
                since: 10,
                stale: false
            }
        );
        input.streaming = false;
        assert_eq!(
            derive(&input).activity,
            Activity::BackgroundWait {
                reason: Wait::Subtasks(2),
                since: 10
            }
        );
        input.wake_tasks = 0;
        assert_eq!(
            derive(&input).activity,
            Activity::BackgroundWait {
                reason: Wait::UsageLimit { resume_at: 100 },
                since: 10
            }
        );
        input.usage_limit_resume_at = None;
        assert_eq!(
            derive(&input).activity,
            Activity::Done {
                outcome: Outcome::Failed,
                at: 9,
                unread: true
            }
        );
        input.seen_at = Some(9);
        assert_eq!(
            derive(&input).activity,
            Activity::Done {
                outcome: Outcome::Failed,
                at: 9,
                unread: false
            }
        );
    }

    #[test]
    fn shell_inventory_never_holds_a_turn_open() {
        let mut input = StatusInput {
            shells: vec![ShellTask {
                id: "dev".into(),
                command: "npm run dev".into(),
            }],
            ..Default::default()
        };
        assert_eq!(derive(&input).activity, Activity::Idle);
        input.last_turn = Some(TurnEnd {
            at: 10,
            outcome: Outcome::Completed,
        });
        let status = derive(&input);
        assert_eq!(
            status.activity,
            Activity::Done {
                outcome: Outcome::Completed,
                at: 10,
                unread: true
            }
        );
        assert_eq!(status.shells, input.shells);
        assert_eq!(status.label(20).as_deref(), Some("Done"));
    }

    #[test]
    fn running_uses_last_event_for_staleness_and_since_for_elapsed() {
        let mut input = StatusInput {
            streaming: true,
            step: Some("Running tests".into()),
            state_since: 500,
            last_event_at: 1_000,
            now: 240_500,
            ..Default::default()
        };
        assert_eq!(
            derive(&input).label(input.now).as_deref(),
            Some("Running tests · 4m")
        );
        input.now = 1_801_000;
        assert_eq!(
            derive(&input).label(input.now).as_deref(),
            Some("No update in 30m")
        );
    }

    #[test]
    fn running_without_an_event_source_is_never_stale() {
        let input = StatusInput {
            streaming: true,
            stale_applicable: false,
            now: STALE_AFTER_MS * 2,
            ..Default::default()
        };
        assert!(matches!(
            derive(&input).activity,
            Activity::Running { stale: false, .. }
        ));
    }
}
