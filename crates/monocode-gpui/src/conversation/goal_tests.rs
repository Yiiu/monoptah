//! The goal shelf derives its state from transcript rows and shares the composer ledge.
use super::*;
use gpui::{TestAppContext, WindowHandle};
use monocode_core::store::Store;
use monocode_core::transcript::{GoalNotice, GoalPhase, Row};
use std::path::PathBuf;
use std::sync::Mutex;

fn conversation(cx: &mut TestAppContext) -> WindowHandle<Conversation> {
    cx.update(|cx| cx.set_reduce_motion(true));
    let store = Store::open(Path::new(":memory:")).unwrap();
    let mut summary = store.create("claude", "/missing/goal-project").unwrap();
    // The composer strip only renders once the session has messages; the
    // /goal echo row makes that true in real sessions too.
    summary.has_messages = true;
    cx.update(|cx| {
        input::bind(cx);
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                view.loaded = true;
                view
            })
        })
        .unwrap()
    })
}

fn goal_row(phase: GoalPhase, condition: Option<&str>, notice: &str) -> Row {
    Row {
        role: "goal".into(),
        text: serde_json::to_string(&GoalNotice {
            phase,
            condition: condition.map(str::to_owned),
            notice: notice.into(),
        })
        .unwrap(),
        first: true,
        sent_at: None,
    }
}

#[gpui::test]
fn goal_state_reads_the_last_row(cx: &mut TestAppContext) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            assert_eq!(view.goal_state(cx), None, "no goal rows means no pill");
            view.transcript.update(cx, |v, cx| {
                v.patch(
                    0,
                    vec![
                        goal_row(GoalPhase::Set, Some("fix it"), "Goal set: fix it"),
                        goal_row(
                            GoalPhase::Checkin,
                            None,
                            "Goal still active · 2 · retrying in 4s",
                        ),
                    ],
                    cx,
                );
            });
            assert_eq!(
                view.goal_state(cx),
                Some((GoalPhase::Set, "fix it".into())),
                "a checkin keeps the pill on the set condition"
            );
            view.transcript.update(cx, |v, cx| {
                v.patch(
                    2,
                    vec![goal_row(GoalPhase::Cleared, None, "No goal set")],
                    cx,
                );
            });
            assert_eq!(view.goal_state(cx), None, "a cleared goal hides the pill");
            view.transcript.update(cx, |v, cx| {
                v.patch(
                    3,
                    vec![
                        goal_row(GoalPhase::Set, Some("fix it"), "Goal set: fix it"),
                        goal_row(
                            GoalPhase::Error,
                            None,
                            "Goal cleared after an unrecoverable error",
                        ),
                    ],
                    cx,
                );
            });
            assert_eq!(view.goal_state(cx), None, "an errored goal hides the pill");
            view.transcript.update(cx, |v, cx| {
                v.patch(
                    5,
                    vec![goal_row(GoalPhase::Paused, None, "Goal paused · waiting")],
                    cx,
                );
            });
            assert_eq!(
                view.goal_state(cx),
                Some((GoalPhase::Paused, "fix it".into())),
                "a paused goal keeps the pill, on the condition it was set with"
            );
        })
        .unwrap();
}

#[gpui::test]
fn goal_shelf_keeps_queue_approval_and_actions_accessible(cx: &mut TestAppContext) {
    let window = conversation(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("goal-card").is_none());
    window
        .update(visual, |view, _, cx| {
            let condition = format!("第一行\r\n{}", "长目标 ".repeat(300));
            let set = goal_row(
                GoalPhase::Set,
                Some(&condition),
                &format!("Goal set: {condition}"),
            );
            let mut history = monocode_core::transcript::Transcript::default();
            history.begin("goal");
            history.append(&set.text);
            assert!(history.rows.len() > 1);
            view.transcript
                .update(cx, |v, cx| v.patch(0, history.rows, cx));
            assert_eq!(
                view.goal_state(cx),
                Some((GoalPhase::Set, condition.clone()))
            );
            let end = view.transcript.read(cx).source_rows().len();
            view.transcript.update(cx, |v, cx| {
                v.patch(
                    end,
                    vec![goal_row(GoalPhase::Paused, None, "Goal paused")],
                    cx,
                )
            });
            assert_eq!(view.goal_state(cx), Some((GoalPhase::Paused, condition)));
            // A pending approval belongs to an active turn. Keep the follow-up
            // queued while its persistence callback runs and the test clicks.
            view.busy = true;
            view.follow_ups.state.paused = true;
            view.enqueue_follow_up("Follow up".into(), None, cx);
            view.approvals.push_back(Approval {
                id: "request".into(),
                title: "Allow this command?".into(),
                detail: String::new(),
                can_remember: true,
            });
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    let card = visual.debug_bounds("goal-card").unwrap();
    let summary = visual.debug_bounds("goal-state").unwrap();
    let clear = visual.debug_bounds("goal-clear").unwrap();
    let expand = visual.debug_bounds("goal-expand").unwrap();
    let queue = visual.debug_bounds("message-queue-card").unwrap();
    let approval = visual.debug_bounds("approval-shelf").unwrap();
    let composer = visual.debug_bounds("composer-box").unwrap();
    assert!(summary.right() <= clear.left() && clear.right() <= expand.left());
    assert!(expand.right() <= card.right());
    assert!(card.bottom() <= queue.top() && queue.bottom() <= approval.top());
    assert!(approval.bottom() <= composer.top());
    let summary_text = visual.debug_bounds("goal-summary-text").unwrap();
    assert!(summary_text.top() >= summary.top() && summary_text.bottom() <= summary.bottom());
    visual.simulate_click(clear.center(), Default::default());
    window
        .update(visual, |view, _, _| {
            assert!(view.busy && view.launched.is_empty());
            assert_eq!(view.approvals.front().unwrap().id, "request");
        })
        .unwrap();
    visual.simulate_click(expand.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
    let details = visual
        .debug_bounds("goal-condition-text")
        .expect("expanded goal details must render");
    assert!(details.size.width > px(0.) && details.size.height > summary_text.size.height);
    window
        .update(visual, |view, _, _| {
            assert!(view.goal_open, "expand opens the full goal condition");
            assert!(view.launched.is_empty(), "expand must not send a prompt");
        })
        .unwrap();
    // From the expand button, Shift-Tab skips the disabled clear button and
    // reaches the summary. Enter and Space activate GPUI's focused buttons on key-up.
    for (forward, key, opens) in [(false, "enter", false), (true, "space", true)] {
        visual.update(|window, cx| {
            if forward {
                window.focus_next(cx)
            } else {
                window.focus_prev(cx)
            }
        });
        visual.refresh().unwrap();
        visual.simulate_keystrokes(key);
        visual.update(|window, cx| {
            let keystroke = gpui::Keystroke::parse(key).unwrap();
            window.dispatch_event(
                gpui::PlatformInput::KeyUp(gpui::KeyUpEvent { keystroke }),
                cx,
            );
        });
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert_eq!(visual.debug_bounds("goal-condition-text").is_some(), opens);
    }
    window
        .update(visual, |view, _, _| assert!(view.launched.is_empty()))
        .unwrap();
}

#[gpui::test]
fn reopening_a_stale_goal_persists_the_completion_and_hides_the_shelf(cx: &mut TestAppContext) {
    struct TempDir(PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = TempDir(std::env::temp_dir().join(monocode_core::mono::new_id()));
    std::fs::create_dir(&dir.0).unwrap();
    let cwd = dir.0.join("project");
    let mut store = Store::open_monocode(&dir.0.join("monocode.db")).unwrap();
    let mut summary = store.create("claude", cwd.to_str().unwrap()).unwrap();
    let session = "4c88026d-46e8-4d1a-aa08-6d18eb537fac";
    store.provider_id(summary.id, session).unwrap();
    summary.provider_id = Some(session.into());
    summary.has_messages = true;
    let settings = monocode_core::agent::Settings {
        account: monocode_core::accounts::Profile::new("goal-test", &dir.0),
        ..Default::default()
    };
    store.save_agent_settings(summary.id, &settings).unwrap();
    let config = settings.account.directory(Kind::Claude).unwrap().unwrap();
    let project =
        config
            .join("projects")
            .join(monocode_core::cloud_sessions::claude_project_dir_name(
                &cwd.to_string_lossy(),
            ));
    std::fs::create_dir_all(&project).unwrap();
    let completion = serde_json::json!({"type":"attachment", "sessionId":session,
        "timestamp":"2026-11-18T08:10:39.628Z", "isSidechain":false,
        "attachment":{"type":"goal_status", "met":true, "condition":"fix it"}});
    std::fs::write(
        project.join(format!("{session}.jsonl")),
        format!("{completion}\n"),
    )
    .unwrap();
    let mut saved = goal_row(GoalPhase::Set, Some("fix it"), "Goal set: fix it");
    saved.sent_at = Some(1_794_988_000_000);
    store.write_rows(summary.id, 0, &[saved]).unwrap();
    let id = summary.id;
    let store = Arc::new(Mutex::new(store));
    let window = cx.update(|cx| {
        cx.set_reduce_motion(true);
        input::bind(cx);
        crate::accounts::install(store.clone(), cx);
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| Conversation::new(summary, store.clone(), cx))
        })
        .unwrap()
    });
    window.update(cx, |view, _, cx| view.load(cx)).unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            assert!(view.loaded);
            assert_eq!(view.goal_state(cx), None);
        })
        .unwrap();
    let rows = store.lock().unwrap().load(id).unwrap().rows;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].role, "goal");
    let completion: GoalNotice = serde_json::from_str(&rows[1].text).unwrap();
    assert_eq!(completion.phase, GoalPhase::Achieved);
}

#[test]
fn late_recovery_checks_the_provider_binding_and_preserves_changed_rows() {
    let mut db = Store::open(Path::new(":memory:")).unwrap();
    let summary = db.create("claude", "/project").unwrap();
    let id = summary.id;
    db.provider_id(id, "original").unwrap();
    let goal = goal_row(GoalPhase::Set, Some("first"), "Goal set: first");
    db.write_rows(id, 0, &[goal]).unwrap();
    let mut history = db.load(id).unwrap();
    let achieved = GoalNotice {
        phase: GoalPhase::Achieved,
        condition: Some("first".into()),
        notice: "Goal achieved".into(),
    };
    let finish = |db: &mut Store, history: &mut monocode_core::transcript::Transcript| {
        history::finish_goal_recovery(
            db,
            id,
            "original",
            "/project",
            &Settings::default(),
            history,
            &achieved,
        )
        .unwrap();
    };
    db.change_provider(id, "codex").unwrap();
    finish(&mut db, &mut history);
    assert_eq!(db.load(id).unwrap().rows.len(), 1);
    db.change_provider(id, "claude").unwrap();
    db.provider_id(id, "other").unwrap();
    finish(&mut db, &mut history);
    assert_eq!(db.load(id).unwrap().rows.len(), 1);
    db.provider_id(id, "original").unwrap();
    db.write_rows(
        id,
        1,
        &[goal_row(GoalPhase::Set, Some("new"), "Goal set: new")],
    )
    .unwrap();
    finish(&mut db, &mut history);
    assert_eq!(db.load(id).unwrap().rows, history.rows);
    assert_eq!(
        GoalNotice::active(&history.rows),
        Some((GoalPhase::Set, "new".into()))
    );
}

#[gpui::test]
fn goal_shelf_clear_launches_the_raw_command(cx: &mut TestAppContext) {
    let window = conversation(cx);
    let view = window;
    window
        .update(cx, |view, _, cx| {
            view.transcript.update(cx, |v, cx| {
                v.patch(
                    0,
                    vec![goal_row(GoalPhase::Set, Some("fix it"), "Goal set: fix it")],
                    cx,
                );
            });
            // The conversation re-renders on its own notify; the runtime's
            // goal event would have delivered one.
            cx.notify();
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.refresh().unwrap();
    visual.run_until_parked();
    let clear = visual
        .debug_bounds("goal-clear")
        .expect("the pill's clear button must exist above the composer");
    visual.simulate_click(clear.center(), Default::default());
    visual.run_until_parked();
    view.update(visual, |view, _, _| {
        assert_eq!(
            view.launched,
            vec!["/goal clear".to_string()],
            "the × sends a raw clear, not an expanded prompt"
        );
    })
    .unwrap();
}
