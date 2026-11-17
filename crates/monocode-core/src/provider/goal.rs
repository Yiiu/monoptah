//! Claude `/goal` lifecycle notices: parse the CLI's synthetic messages into
//! typed state. Version-coupled to the claude CLI (verified 2.1.311/2.1.325);
//! the notice→phase table lives only here.
use crate::transcript::GoalPhase;

/// Map a synthetic notice line to a goal state change, if it is one.
pub fn goal_notice(text: &str) -> Option<crate::transcript::GoalNotice> {
    let line = text.trim();
    let (phase, condition) = if let Some(rest) = line.strip_prefix("Goal set: ") {
        (GoalPhase::Set, Some(rest.trim().to_owned()))
    } else if line == "No goal set" {
        (GoalPhase::Cleared, None)
    } else if line.starts_with("Goal cleared after an unrecoverable error") {
        (GoalPhase::Error, None)
    } else if let Some(rest) = line.strip_prefix("Goal cleared: ") {
        (GoalPhase::Cleared, Some(rest.trim().to_owned()))
    } else if line.starts_with("Goal achieved") {
        (GoalPhase::Achieved, None)
    } else if line.starts_with("Goal still active") {
        (GoalPhase::Checkin, None)
    } else if line.starts_with("Goal paused") || line.starts_with("Goal could not be achieved") {
        (GoalPhase::Paused, None)
    } else {
        return None;
    };
    Some(crate::transcript::GoalNotice {
        phase,
        condition,
        notice: line.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::GoalNotice;

    #[test]
    fn parses_every_notice_in_the_transition_table() {
        let set = goal_notice("Goal set: reply with ok").unwrap();
        assert_eq!(set.phase, GoalPhase::Set);
        assert_eq!(set.condition.as_deref(), Some("reply with ok"));
        assert_eq!(
            goal_notice("No goal set").map(|g| g.phase),
            Some(GoalPhase::Cleared)
        );
        assert_eq!(
            goal_notice("Goal cleared: reply with ok").map(|g| g.phase),
            Some(GoalPhase::Cleared)
        );
        assert_eq!(
            goal_notice("Goal achieved").map(|g| g.phase),
            Some(GoalPhase::Achieved)
        );
        assert_eq!(
            goal_notice("Goal still active · 2 · retrying in 4s").map(|g| g.phase),
            Some(GoalPhase::Checkin)
        );
        assert_eq!(
            goal_notice("Goal paused · the last turn could not finish · x").map(|g| g.phase),
            Some(GoalPhase::Paused)
        );
        assert_eq!(
            goal_notice("Goal could not be achieved").map(|g| g.phase),
            Some(GoalPhase::Paused)
        );
        let err = goal_notice(
            "Goal cleared after an unrecoverable error (2): boom. Run /goal again to continue.",
        )
        .unwrap();
        assert_eq!(err.phase, GoalPhase::Error);
    }

    #[test]
    fn cleared_notices_carry_the_condition() {
        let cleared = goal_notice("Goal cleared: fix the login flow").unwrap();
        assert_eq!(cleared.phase, GoalPhase::Cleared);
        assert_eq!(cleared.condition.as_deref(), Some("fix the login flow"));
    }

    #[test]
    fn unknown_text_is_not_a_goal_notice() {
        assert!(goal_notice("Here is your summary").is_none());
        assert!(goal_notice("").is_none());
        // Informational only: no state change.
        assert!(goal_notice("Goal condition is limited to 500 characters").is_none());
    }

    #[test]
    fn notice_keeps_the_full_line() {
        let line = "Goal set: ship the release";
        let notice: GoalNotice = goal_notice(line).unwrap();
        assert_eq!(notice.notice, line);
    }
}
