use super::*;
use crate::transcript::Row;
use std::path::Path;

const CWD: &str = "/workspace";

fn completed(at: u64) -> TurnEnd {
    TurnEnd {
        at,
        outcome: Outcome::Completed,
    }
}

#[test]
fn summaries_only_change_the_matching_finished_turn() {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let id = store.create("claude", CWD).unwrap().id;
    store.restore_session_activity(10).unwrap();
    store
        .finish_session_turn_with_summary(id, 20, completed(30), Some("First fallback"))
        .unwrap();
    assert!(store
        .update_progress_summary(id, 30, "First generated")
        .unwrap());
    assert_eq!(
        store.restore_session_activity(35).unwrap()[&id]
            .progress_summary
            .as_deref(),
        Some("First generated")
    );

    store.begin_session_turn(id, 40).unwrap();
    assert!(!store.update_progress_summary(id, 30, "Stale").unwrap());
    store
        .finish_session_turn_with_summary(id, 40, completed(50), Some("Second fallback"))
        .unwrap();
    assert!(!store.update_progress_summary(id, 30, "Stale").unwrap());
    assert_eq!(
        store.restore_session_activity(60).unwrap()[&id]
            .progress_summary
            .as_deref(),
        Some("Second fallback")
    );
}

#[test]
fn restoring_a_session_keeps_its_consumed_worktree_event() {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let id = store.create("codex", CWD).unwrap().id;
    store.restore_session_activity(10).unwrap();
    store.set_archived(id, true).unwrap();
    store.remember_worktree_archive(id, "removed:test").unwrap();
    store.set_archived(id, false).unwrap();
    assert!(!store.sessions().unwrap()[0].archived);
    assert_eq!(
        store.restore_session_activity(20).unwrap()[&id]
            .auto_archive_key
            .as_deref(),
        Some("removed:test")
    );
}

#[test]
fn migration_marks_existing_history_read_once_and_leaves_drafts_idle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("monocode.db");
    let mut store = Store::open_monocode(&path).unwrap();
    let existing = store.create("claude", CWD).unwrap().id;
    store
        .write_rows(
            existing,
            0,
            &[Row {
                role: "user".into(),
                text: "Existing prompt".into(),
                first: true,
                sent_at: Some(10),
            }],
        )
        .unwrap();
    let draft = store.create("claude", CWD).unwrap().id;
    store
        .conn
        .execute("UPDATE sessions SET updated_at=40", [])
        .unwrap();
    let original_schema: i64 = store
        .conn
        .query_row("PRAGMA schema_version", [], |row| row.get(0))
        .unwrap();

    let migrated = store.restore_session_activity(100).unwrap();
    assert_eq!(migrated[&existing].last_turn, Some(completed(40)));
    assert_eq!(migrated[&existing].seen_at, Some(100));
    assert_eq!(migrated[&draft].last_turn, None);
    assert_eq!(migrated[&draft].seen_at, Some(100));
    assert_eq!(
        store
            .conn
            .query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        original_schema
    );

    store
        .finish_session_turn(existing, 150, completed(200))
        .unwrap();
    let new_draft = store.create("claude", CWD).unwrap().id;
    drop(store);

    let store = Store::open_monocode(&path).unwrap();
    let restored = store.restore_session_activity(300).unwrap();
    assert_eq!(restored[&existing].last_turn, Some(completed(200)));
    assert_eq!(restored[&existing].seen_at, Some(100));
    assert!(!restored.contains_key(&new_draft));
}

#[test]
fn table_recreation_restores_unread_receipts_after_restart_without_backfilling() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("monocode.db");
    let store = Store::open_monocode(&path).unwrap();
    let unread = store.create("claude", CWD).unwrap().id;
    let read = store.create("claude", CWD).unwrap().id;
    store.restore_session_activity(100).unwrap();
    store
        .finish_session_turn_with_summary(unread, 110, completed(120), Some("Keep me"))
        .unwrap();
    store
        .finish_session_turn(read, 110, completed(130))
        .unwrap();
    store.see_session_turn(read, 130, 140).unwrap();
    let untouched = store.create("claude", CWD).unwrap().id;
    store
        .native_db()
        .unwrap()
        .execute_batch("DROP TABLE session_activity")
        .unwrap();
    drop(store);

    let reopened = Store::open_monocode(&path).unwrap();
    let migrated = reopened.restore_session_activity(200).unwrap();
    assert_eq!(migrated[&unread].last_turn, Some(completed(120)));
    assert_eq!(migrated[&unread].seen_at, Some(100));
    assert_eq!(
        migrated[&unread].progress_summary.as_deref(),
        Some("Keep me")
    );
    assert_eq!(migrated[&read].last_turn, Some(completed(130)));
    assert_eq!(migrated[&read].seen_at, Some(130));
    assert!(
        !migrated.contains_key(&untouched),
        "repair does not backfill new sessions"
    );
}

#[test]
fn live_repair_retains_active_owners_read_summaries_and_unread_status() {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let unread = store.create("claude", CWD).unwrap().id;
    let read = store.create("claude", CWD).unwrap().id;
    let active = store.create("claude", CWD).unwrap().id;
    store.restore_session_activity(100).unwrap();
    store
        .finish_session_turn(unread, 110, completed(120))
        .unwrap();
    store
        .finish_session_turn_with_summary(read, 110, completed(130), Some("Reviewed"))
        .unwrap();
    store.see_session_turn(read, 130, 140).unwrap();
    store.begin_session_turn(active, 130).unwrap();
    let before = store.restore_session_activity(140).unwrap();
    store.conn.execute_batch("DROP TABLE session_activity; CREATE TABLE session_activity (session_id TEXT PRIMARY KEY)").unwrap();

    assert!(store.see_session_turn(unread, 120, 150).is_err());
    assert!(store.repair_session_activity(140).unwrap());
    assert!(!store.repair_session_activity(150).unwrap());
    assert_eq!(store.restore_session_activity(150).unwrap(), before);
    store
        .finish_session_turn(active, 125, completed(150))
        .unwrap();
    assert_eq!(
        store.restore_session_activity(150).unwrap()[&active].last_turn,
        None
    );
    store
        .finish_session_turn(active, 130, completed(150))
        .unwrap();
    store.see_session_turn(unread, 120, 150).unwrap();
    store
        .conn
        .execute_batch("DROP TABLE session_activity")
        .unwrap();
    let saved = store.restore_session_activity(160).unwrap();
    assert_eq!(saved[&unread].seen_at, Some(120));
    assert_eq!(saved[&read], before[&read]);
    assert_eq!(saved[&active].last_turn, Some(completed(150)));
}

#[test]
fn out_of_order_turn_writes_and_acknowledgements_keep_the_latest_completion_unread() {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let id = store.create("claude", CWD).unwrap().id;
    store.restore_session_activity(10).unwrap();

    // The background writer may get the acknowledgement before its completion.
    store.see_session_turn(id, 30, 40).unwrap();
    store.finish_session_turn(id, 20, completed(30)).unwrap();
    store.begin_session_turn(id, 20).unwrap();
    let restored = store.restore_session_activity(50).unwrap();
    assert_eq!(restored[&id].last_turn, Some(completed(30)));
    assert_eq!(restored[&id].seen_at, Some(30));

    store.begin_session_turn(id, 60).unwrap();
    store.finish_session_turn(id, 60, completed(70)).unwrap();
    store.finish_session_turn(id, 20, completed(90)).unwrap();
    store.see_session_turn(id, 30, 100).unwrap();
    let restored = store.restore_session_activity(110).unwrap();
    assert_eq!(restored[&id].last_turn, Some(completed(70)));
    assert_eq!(restored[&id].seen_at, Some(30));
}

#[test]
fn restart_recovers_only_orphaned_local_turns_and_keeps_the_result_unread() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("monocode.db");
    let store = Store::open_monocode(&path).unwrap();
    let orphaned = store.create("claude", CWD).unwrap().id;
    let live = store.create("claude", CWD).unwrap().id;
    store.restore_session_activity(100).unwrap();
    store.begin_session_turn(orphaned, 120).unwrap();
    store.begin_session_turn(live, 130).unwrap();
    // A persisted owner of zero is never a live Native process.
    store
        .native_db()
        .unwrap()
        .execute(
            "UPDATE session_activity SET owner_pid=0 WHERE session_id=?1",
            [store.persisted_session_id(orphaned).unwrap()],
        )
        .unwrap();
    drop(store);

    let store = Store::open_monocode(&path).unwrap();
    let restored = store.restore_session_activity(200).unwrap();
    let stopped = TurnEnd {
        at: 200,
        outcome: Outcome::Interrupted,
    };
    assert_eq!(restored[&orphaned].last_turn, Some(stopped));
    assert_eq!(restored[&orphaned].seen_at, Some(100));
    assert_eq!(restored[&live].last_turn, None);

    store
        .finish_session_turn(orphaned, 120, completed(250))
        .unwrap();
    store.see_session_turn(orphaned, 200, 220).unwrap();
    let restored = store.restore_session_activity(300).unwrap();
    assert_eq!(restored[&orphaned].last_turn, Some(stopped));
    assert_eq!(restored[&orphaned].seen_at, Some(200));
}

#[test]
fn deleting_a_session_clears_its_activity_even_when_prototype_ids_are_reused() {
    let mut store = Store::open(Path::new(":memory:")).unwrap();
    let deleted = store.create("claude", CWD).unwrap().id;
    store.restore_session_activity(100).unwrap();
    store
        .finish_session_turn(deleted, 110, completed(120))
        .unwrap();

    store.delete(deleted).unwrap();
    let new = store.create("claude", CWD).unwrap().id;
    assert_eq!(new, deleted);
    assert!(!store
        .restore_session_activity(150)
        .unwrap()
        .contains_key(&new));
}
