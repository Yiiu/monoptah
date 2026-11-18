//! The goal pill above the composer: its state derives from the transcript's
//! goal rows, and its × launches a raw clear command.
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
fn reopening_a_stale_goal_persists_the_completion_and_hides_the_pill(cx: &mut TestAppContext) {
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

#[gpui::test]
fn goal_pill_clear_launches_the_raw_command(cx: &mut TestAppContext) {
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
