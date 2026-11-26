//! Task 3 tests: the skeleton+chunks round trip, oversized-row chunking, the
//! open-time write barrier, and unchanged blob behavior for marker-less
//! sessions. Task 4 tests: the sparse block-level writer's dirty-set
//! discipline, revision conflicts, receipt transactionality and refusals.
use super::layout::{
    block_raw_json, block_skeleton, block_value, pack_payload_segments, pack_row_chunks,
    pack_stored_chunks, store_block_form, PAYLOAD_PLACEHOLDER, ROWS_MEMBER, ROWS_PLACEHOLDER,
    TEXT_SENTINEL,
};
use super::writer::{
    flush_sparse, BlockUpdate, DirtyBlock, FlushConflict, FlushError, SparseFlush, SparseSavePlan,
    CHUNK_TARGET_BYTES,
};
use super::*;
use crate::store::shared::{blocks, handle};
use crate::store::Store;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

/// The probe's fixed logical time: typed, so `json!` keeps it a u64.
const FIXED_TIME: u64 = 1_700_000_000_000;

/// The probe's block fingerprint: a SHA-256 of the serialized block set.
fn json_hash(value: &impl serde::Serialize) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap()))
}

fn insert_session(conn: &Connection, id: &str, blocks: &[Value]) {
    conn.execute(
        "INSERT INTO sessions(id,cwd,harness,model,runtime_mode,title,blocks_json,created_at,updated_at,has_user_message,queued_messages_json)
         VALUES(?1,'synthetic','synthetic','synthetic:fixed','supervised','Synthetic',?2,?3,?3,1,'[]')",
        params![id, serde_json::to_string(blocks).unwrap(), 1_700_000_000_i64],
    )
    .unwrap();
}

/// The stored document minus its envelope member is the desktop block again.
fn strip_rows_member(raw: &str) -> Value {
    let mut document: Value = serde_json::from_str(raw).unwrap();
    assert!(document
        .as_object_mut()
        .unwrap()
        .remove(ROWS_MEMBER)
        .is_some());
    document
}

#[test]
fn reading_agent_settings_does_not_reconstruct_ordinary_history_blocks() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

    let store = Store::open_monocode(Path::new(":memory:")).unwrap();
    seed_native_session(
        &store.conn,
        "metadata-only",
        &[json!({
            "id":"output", "role":"tool", "text":"Command output",
            "tool":{"kind":"execute", "nativeInput":"synthetic argument".repeat(10_000)},
        })],
    );
    let id = store.resolve_session_id("metadata-only").unwrap();
    store
        .conn
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "native_blocks",
                column_name: "skeleton_json",
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }))
        .unwrap();

    let settings = store.agent_settings(id).unwrap().unwrap();
    assert!(settings.turns.is_empty());
    assert!(!settings.operator_enabled);
}

#[test]
fn chunked_agent_settings_match_blob_turns_and_operator_history() {
    let store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let blocks = [
        json!({"id":"lead", "role":"assistant", "text":"Earlier reply"}),
        json!({
            "id":"prompt", "role":"user", "text":"多行🙂 ".repeat(350),
            "attachments":[{"id":"file", "path":"synthetic/example.txt", "kind":"file"}],
            "startedAt":FIXED_TIME, "durationMs":850, "providerTurnId":"historical-turn",
            "turnModel":{"harness":"codex","id":"codex:old","name":"Old model"},
            "turnMetrics":{"input_tokens":500,"output_tokens":22},
        }),
        json!({"id":"draft", "role":"user", "text":"/mono", "draft":true}),
        json!({"id":"tool", "role":"tool", "text":"Tool detail","tool":{"kind":"read"}}),
        json!({"id":"steer", "role":"user", "text":"Continue", "sentAt":FIXED_TIME + 900}),
        json!({
            "id":"operator", "role":"user", "text":"Operator notice", "monocode":true,
            "interjection":{"customType":"custom"},
        }),
        json!({
            "id":"internal", "role":"user", "text":"Monitor finished", "internal":true,
            "nativeStopped":true,
        }),
    ];
    insert_session(&store.conn, "legacy-settings-blob", &blocks);
    let blob_id = store.resolve_session_id("legacy-settings-blob").unwrap();
    let expected = store.agent_settings(blob_id).unwrap().unwrap();
    assert!(expected.operator_enabled);
    assert_eq!(expected.turns.len(), 3);
    assert!(expected
        .turns
        .values()
        .any(|turn| turn.joined_turn.is_some()));
    for (name, version) in [("legacy-settings-chunks", 1), ("settings-chunks", 2)] {
        seed_native_session(&store.conn, name, &blocks);
        store
            .conn
            .execute(
                "UPDATE native_sessions SET mode_version=?1 WHERE session_id=?2",
                params![version, name],
            )
            .unwrap();
        // Row role is still correct with a noncanonical prefix.
        store
            .conn
            .execute(
                "UPDATE native_chunks SET rows_json=' ' || rows_json, byte_len=byte_len+1
                 WHERE session_id=?1 AND block_seq=4 AND chunk_ord=0",
                [name],
            )
            .unwrap();
        store
            .conn
            .execute(
                r#"UPDATE native_chunks
                   SET rows_json=replace(rows_json, '"role":"user"', ?2),
                       byte_len=length(CAST(replace(rows_json, '"role":"user"', ?2) AS BLOB))
                   WHERE session_id=?1 AND block_seq=1 AND chunk_ord=0"#,
                params![name, r#""role":"\u0075ser""#],
            )
            .unwrap();
        let chunked_id = store.resolve_session_id(name).unwrap();
        let actual = store.agent_settings(chunked_id).unwrap().unwrap();
        assert_eq!(actual.operator_enabled, expected.operator_enabled);
        assert_eq!(
            serde_json::to_value(actual.turns).unwrap(),
            serde_json::to_value(&expected.turns).unwrap(),
        );
    }
}

#[test]
fn metadata_only_user_blocks_still_record_operator_usage() {
    let store = Store::open_monocode(Path::new(":memory:")).unwrap();
    seed_native_session(
        &store.conn,
        "metadata-operator",
        &[json!({
            "id":"operator-card", "role":"user", "text":"", "monocode":true,
            "monoCard":{"type":"document","documentId":"synthetic-doc","title":"Synthetic card"},
        })],
    );
    let id = store.resolve_session_id("metadata-operator").unwrap();
    assert!(store.agent_settings(id).unwrap().unwrap().operator_enabled);
}

#[test]
fn chunked_preview_uses_absolute_turn_boundaries_and_keeps_full_history() {
    let store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let mut blocks = vec![json!({"id":"notice","role":"system","text":"Preamble"})];
    for turn in 0..5 {
        blocks.extend([
            json!({"id":format!("prompt-{turn}"),"role":"user","text":format!("Prompt {turn}")}),
            json!({"id":format!("reply-{turn}"),"role":"assistant","text":format!("Reply {turn}")}),
        ]);
    }
    seed_native_session(&store.conn, "preview-chunks", &blocks);
    let id = store.resolve_session_id("preview-chunks").unwrap();
    let all = blocks::project(&blocks).unwrap();

    let tail = store.chunked_history_window(id, None, 2).unwrap().unwrap();
    assert_eq!(tail, (7, all[7..].to_vec()));
    let anchored = store
        .chunked_history_window(id, Some(6), 2)
        .unwrap()
        .unwrap();
    assert_eq!(anchored, (3, all[3..7].to_vec()));
    let preamble = store
        .chunked_history_window(id, Some(0), 1)
        .unwrap()
        .unwrap();
    assert_eq!(preamble, (0, all[..3].to_vec()));
    let anchored_tail = store
        .chunked_history_window(id, Some(10), 3)
        .unwrap()
        .unwrap();
    assert_eq!(anchored_tail, (5, all[5..].to_vec()));
    assert!(store
        .chunked_history_window(id, Some(all.len()), 2)
        .unwrap()
        .is_none());
    assert_eq!(store.load(id).unwrap().rows, all);
    assert_eq!(
        session_revision(&store.conn, "preview-chunks").unwrap(),
        Some(0)
    );
    for table in ["native_blocks", "native_chunks"] {
        store
            .conn
            .execute(
                &format!("UPDATE {table} SET block_seq=block_seq*100+50 WHERE session_id=?1"),
                ["preview-chunks"],
            )
            .unwrap();
    }
    assert_eq!(
        store.chunked_history_window(id, Some(6), 2).unwrap(),
        Some(anchored)
    );

    insert_session(&store.conn, "preview-blob", &blocks);
    let blob_id = store.resolve_session_id("preview-blob").unwrap();
    assert!(store
        .chunked_history_window(blob_id, None, 2)
        .unwrap()
        .is_none());
}

#[test]
fn reading_an_early_anchor_skips_unrelated_later_chunk_payloads() {
    let store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let mut blocks = Vec::new();
    for turn in 0..200 {
        blocks.extend([
            json!({"id":format!("prompt-{turn}"),"role":"user","text":format!("Prompt {turn}")}),
            json!({"id":format!("reply-{turn}"),"role":"assistant","text":format!("Reply {turn}")}),
        ]);
    }
    let all = blocks::project(&blocks).unwrap();
    seed_native_session(&store.conn, "early-anchor", &blocks);
    let id = store.resolve_session_id("early-anchor").unwrap();
    // A whole-history or newest-first payload scan would hit this malformed
    // chunk long before reaching the saved reading position.
    store
        .conn
        .execute(
            "UPDATE native_chunks SET rows_json='invalid', byte_len=7
             WHERE session_id=?1 AND block_seq=300 AND chunk_ord=0",
            ["early-anchor"],
        )
        .unwrap();
    assert_eq!(
        store.chunked_history_window(id, Some(7), 2).unwrap(),
        Some((4, all[4..8].to_vec()))
    );
    assert!(store.page_rows(id, 300, 1).is_err());
}

#[test]
fn goal_rows_keep_their_role_when_loading_old_native_chunks() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    // Legacy goals can have pretty-printed JSON and arbitrary field ordering.
    // The body may even contain reserved markers that use a full-block envelope.
    let text = r#"{ "condition": "finish", "phase" : "set", "notice": "finish" }"#;
    let blocks = [
        json!({"id":"goal","role":"goal","text":text,"sentAt":FIXED_TIME}),
        json!({"id":"system","role":"system","text":text}),
        json!({"id":"full-goal","role":"goal","text":text,"extra":ROWS_PLACEHOLDER}),
    ];
    assert_eq!(blocks::project(&blocks).unwrap()[0].role, "goal");
    seed_native_session(&store.conn, "old-goal", &blocks);
    // Older writers preserved the outer block role but projected a system row.
    store.conn.execute(
        "UPDATE native_chunks SET rows_json=replace(rows_json,'\"role\":\"goal\"','\"role\":\"system\"'), byte_len=byte_len+2 WHERE session_id='old-goal' AND block_seq IN (0,2)",
        [],
    ).unwrap();
    let rows = page_rows(&store.conn, "old-goal", 0, 20).unwrap();
    assert_eq!(rows[0].role, "goal");
    assert_eq!(rows[0].sent_at, Some(FIXED_TIME));
    assert_eq!(rows[1].role, "system");
    assert_eq!(rows[2].role, "goal");
    let id = store.resolve_session_id("old-goal").unwrap();
    store.downgrade_session_to_blob(id).unwrap();
    assert_eq!(store.stored_blocks(id).unwrap(), blocks);
    assert_eq!(store.load(id).unwrap().rows, rows);
}

#[test]
fn round_trip_preserves_unknown_fields_and_hidden_blocks() {
    let blocks = vec![
        json!({"id":"fixed-prompt","role":"user","text":"中文🙂 synthetic prompt","startedAt":FIXED_TIME,
            "appRequestId":"fixed-delivered","providerTurnId":"fixed-turn","durationMs":650,
            "turnModel":{"harness":"synthetic","id":"synthetic:fixed","name":"Synthetic"},
            "attachments":[{"id":"fixed-file","path":"synthetic/fixture.txt","kind":"file","mimeType":"text/plain","size":10,"futureAttachment":true}],
            "futureMetadata":{"fixed":[1,2,3]}}),
        json!({"id":"hidden-draft","role":"assistant","text":"never displayed","draft":true,"futureHidden":"keep"}),
        json!({"id":"internal","role":"user","internal":true,"text":"Monitor finished","sentAt":123,"future":true}),
        json!({"id":"fixed-tool","role":"tool","text":"Synthetic tool","sentAt":FIXED_TIME,
            "tool":{"callId":"fixed-call","nativeName":"Read","nativeInput":"{}","kind":"read","title":"Synthetic tool","status":"completed","detail":"synthetic output","futureToolField":[1,2]},"futureMetadata":true}),
        json!({"id":"fixed-interjection","role":"system","text":"Synthetic notice","sentAt":1,
            "interjection":{"customType":"custom","severity":"info"},"futureMetadata":7}),
        json!({"id":"fixed-card","role":"assistant","text":"","monoCard":{"type":"document","documentId":"fixed-document","title":"Synthetic card","futureCard":true},"futureMetadata":9}),
    ];
    // A new database gets the chunked-backend tables from the main opener.
    let store = Store::open_monocode(Path::new(":memory:")).unwrap();
    store
        .conn
        .execute(
            "INSERT INTO native_sessions(session_id,mode_version,session_revision) VALUES('round-trip',?1,0)",
            params![NATIVE_CHUNKS_MODE_VERSION as i64],
        )
        .unwrap();
    assert_eq!(
        session_storage_mode(&store.conn, "round-trip").unwrap(),
        StorageMode::NativeChunks {
            mode_version: NATIVE_CHUNKS_MODE_VERSION
        }
    );
    let mut first_row = 0_i64;
    let mut all_elements = Vec::new();
    for (seq, block) in blocks.iter().enumerate() {
        let raw = serde_json::to_string(block).unwrap();
        let rows = blocks::project(std::slice::from_ref(block)).unwrap();
        let (skeleton, elements) = block_skeleton(&raw, &rows).unwrap();
        let chunks = pack_row_chunks(&elements, 64);
        store
            .conn
            .execute(
                "INSERT INTO native_blocks(session_id,block_seq,block_id,skeleton_json,first_row,row_count,block_revision)
                 VALUES('round-trip',?1,?2,?3,?4,?5,0)",
                params![seq as i64, block["id"].as_str(), skeleton, first_row, rows.len() as i64],
            )
            .unwrap();
        for (ord, chunk) in chunks.iter().enumerate() {
            store
                .conn
                .execute(
                    "INSERT INTO native_chunks(session_id,block_seq,chunk_ord,rows_json,byte_len)
                     VALUES('round-trip',?1,?2,?3,?4)",
                    params![seq as i64, ord as i64, chunk, chunk.len() as i64],
                )
                .unwrap();
        }
        first_row += rows.len() as i64;
        all_elements.push(elements);
    }
    // Stable absolute row numbers: the projected rows land in first_row/row_count.
    assert_eq!(first_row, 6);
    // A hidden block projects no rows, stores no chunks, and its skeleton is
    // its raw JSON plus the empty rows member — pinned byte for byte.
    let draft_raw = serde_json::to_string(&blocks[1]).unwrap();
    let (draft_skeleton, draft_elements) = block_skeleton(&draft_raw, &[]).unwrap();
    assert!(draft_elements.is_empty());
    assert_eq!(
        draft_skeleton,
        format!(
            "{},\"{ROWS_MEMBER}\":[{ROWS_PLACEHOLDER}]}}",
            &draft_raw[..draft_raw.len() - 1]
        )
    );
    assert_eq!(
        block_raw_json(&draft_skeleton, &[], &[]).unwrap(),
        format!(
            "{},\"{ROWS_MEMBER}\":[]}}",
            &draft_raw[..draft_raw.len() - 1]
        )
    );
    let hidden: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM native_chunks WHERE session_id='round-trip' AND block_seq=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hidden, 0);
    // Reconstruct in block order, splicing the stored chunks back in.
    let mut reconstructed = Vec::new();
    let mut statement = store
        .conn
        .prepare("SELECT block_seq,skeleton_json FROM native_blocks WHERE session_id='round-trip' ORDER BY block_seq")
        .unwrap();
    let stored = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    drop(statement);
    for (seq, skeleton) in stored {
        let mut chunks_query = store
            .conn
            .prepare("SELECT rows_json FROM native_chunks WHERE session_id='round-trip' AND block_seq=?1 ORDER BY chunk_ord")
            .unwrap();
        let chunks = chunks_query
            .query_map(params![seq], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        drop(chunks_query);
        let raw = block_raw_json(&skeleton, &chunks, &[]).unwrap();
        // Chunk boundaries never change the reconstruction.
        assert_eq!(
            raw,
            block_raw_json(
                &skeleton,
                &pack_row_chunks(&all_elements[seq as usize], 64 * 1024),
                &[]
            )
            .unwrap()
        );
        reconstructed.push(strip_rows_member(&raw));
    }
    assert_eq!(reconstructed, blocks);
    assert_eq!(json_hash(&reconstructed), json_hash(&blocks));
    // The reserved envelope member and placeholder refuse to be smuggled.
    assert!(block_skeleton(
        &serde_json::to_string(&json!({"id":"x","rows":[]})).unwrap(),
        &[]
    )
    .is_err());
    assert!(block_skeleton(
        &serde_json::to_string(
            &json!({"id":"x","role":"user","text":"carry %NATIVE_ROWS% inside"})
        )
        .unwrap(),
        &[]
    )
    .is_err());
    assert!(block_raw_json("no placeholder here", &[], &[]).is_err());
    assert!(block_raw_json("%NATIVE_ROWS% and %NATIVE_ROWS%", &[], &[]).is_err());
}

#[test]
fn oversized_row_is_single_chunk() {
    // The packing rule: chunks break at row boundaries only, and a row larger
    // than the target is one chunk on its own.
    let a = "a".repeat(30);
    let b = "b".repeat(30);
    let c = "c".repeat(30);
    // 30 + 1 + 30 = 61 fits the target exactly; one byte less splits the pair.
    assert_eq!(
        pack_row_chunks(&[a.clone(), b.clone(), c.clone()], 61),
        vec![format!("{a},{b}"), c.clone()]
    );
    assert_eq!(
        pack_row_chunks(&[a, b, c], 60),
        vec!["a".repeat(30), "b".repeat(30), "c".repeat(30)]
    );
    let oversized = "x".repeat(200);
    let chunks = pack_row_chunks(&["s".repeat(20), oversized.clone(), "t".repeat(20)], 64);
    assert_eq!(
        chunks
            .iter()
            .filter(|chunk| chunk.contains(&oversized))
            .count(),
        1
    );
    assert!(chunks
        .iter()
        .all(|chunk| !chunk.contains(&oversized) || chunk == &oversized));
    assert_eq!(chunks, vec!["s".repeat(20), oversized, "t".repeat(20)]);
    // No rows means no chunks: the rows array splices back empty.
    assert!(pack_row_chunks(&[], 64).is_empty());

    // A real long answer through the tables: the rich adapter virtualizes it
    // into multiple rows, and every row larger than the target is a chunk of
    // its own, with byte_len pinning each stored width.
    let store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let block = json!({"id":"long-answer","role":"assistant","text":"x".repeat(4096),"sentAt":1,"futureMetadata":true});
    let rows = blocks::project(std::slice::from_ref(&block)).unwrap();
    assert_eq!(rows.len(), 2);
    let raw = serde_json::to_string(&block).unwrap();
    let (skeleton, elements) = block_skeleton(&raw, &rows).unwrap();
    assert!(elements.iter().all(|element| element.len() > 64));
    let chunks = pack_row_chunks(&elements, 64);
    assert_eq!(chunks, elements);
    store
        .conn
        .execute(
            "INSERT INTO native_blocks(session_id,block_seq,block_id,skeleton_json,first_row,row_count,block_revision)
             VALUES('oversized',0,'long-answer',?1,0,?2,0)",
            params![skeleton, rows.len() as i64],
        )
        .unwrap();
    for (ord, chunk) in chunks.iter().enumerate() {
        store
            .conn
            .execute(
                "INSERT INTO native_chunks(session_id,block_seq,chunk_ord,rows_json,byte_len)
                 VALUES('oversized',0,?1,?2,?3)",
                params![ord as i64, chunk, chunk.len() as i64],
            )
            .unwrap();
    }
    let (stored_skeleton, stored_json, stored_len): (String, String, i64) = store
        .conn
        .query_row(
            "SELECT b.skeleton_json,c.rows_json,c.byte_len FROM native_blocks b
             JOIN native_chunks c ON c.session_id=b.session_id AND c.block_seq=b.block_seq
             WHERE b.session_id='oversized' AND c.chunk_ord=0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(stored_len, stored_json.len() as i64);
    assert_eq!(stored_json, elements[0]);
    let mut stored_chunks = vec![stored_json];
    stored_chunks.push(chunks[1].clone());
    assert_eq!(
        strip_rows_member(&block_raw_json(&stored_skeleton, &stored_chunks, &[]).unwrap()),
        block
    );
}

#[test]
fn older_writer_refuses_newer_mode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("barrier.db");
    {
        // A marker this build understands does not block writes.
        let mut store = Store::open_monocode(&path).unwrap();
        insert_session(
            &store.conn,
            "current",
            &[json!({"id":"c","role":"user","text":"Current format","sentAt":1})],
        );
        store
            .conn
            .execute(
                "INSERT INTO native_sessions(session_id,mode_version,session_revision) VALUES('current',?1,0)",
                params![NATIVE_CHUNKS_MODE_VERSION as i64],
            )
            .unwrap();
        let id = store.resolve_session_id("current").unwrap();
        let transcript = store.load(id).unwrap();
        store.write_rows(id, 0, &transcript.rows).unwrap();
    }
    {
        // A newer binary leaves a marker this build cannot understand.
        let store = Store::open_monocode(&path).unwrap();
        insert_session(
            &store.conn,
            "future",
            &[json!({"id":"f","role":"user","text":"Future format","sentAt":1})],
        );
        store
            .conn
            .execute(
                "INSERT INTO native_sessions(session_id,mode_version,session_revision) VALUES('future',?1,0)",
                params![i64::from(NATIVE_CHUNKS_MODE_VERSION) + 1],
            )
            .unwrap();
    }
    // Opening still succeeds: supported sessions stay readable, future
    // representations refuse both reads and writes.
    let mut store = Store::open_monocode(&path).unwrap();
    let error = session_storage_mode(&store.conn, "future")
        .unwrap_err()
        .to_string();
    assert!(error.contains(UNSUPPORTED_STORAGE_MODE), "{error}");
    let id = store.resolve_session_id("future").unwrap();
    let error = store.load(id).unwrap_err();
    assert!(error.contains(UNSUPPORTED_STORAGE_MODE), "{error}");
    let error = store.write_rows(id, 0, &[]).unwrap_err();
    assert!(error.contains(UNSUPPORTED_STORAGE_MODE), "{error}");
    let error = store.save_setting("blocked", "1").unwrap_err();
    assert!(error.contains(UNSUPPORTED_STORAGE_MODE), "{error}");
    // The whole connection refuses, even for the session this build understands.
    let current = store.resolve_session_id("current").unwrap();
    let rows = store.load(current).unwrap().rows;
    let error = store.write_rows(current, 0, &rows).unwrap_err();
    assert!(error.contains(UNSUPPORTED_STORAGE_MODE), "{error}");

    // A future whole-database marker is just another row in the same table.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("whole-db.db");
    {
        let store = Store::open_monocode(&path).unwrap();
        store
            .conn
            .execute(
                "INSERT INTO native_sessions(session_id,mode_version,session_revision) VALUES('__database__',?1,0)",
                params![i64::from(NATIVE_CHUNKS_MODE_VERSION) + 1],
            )
            .unwrap();
    }
    let store = Store::open_monocode(&path).unwrap();
    let error = store.save_setting("blocked", "1").unwrap_err();
    assert!(error.contains(UNSUPPORTED_STORAGE_MODE), "{error}");
}

fn mode_flip_refuses_stale_write(native_before: bool, full_replace: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mode-flip.db");
    let original = "mode-flip";
    let blocks = vec![json!({"id":"prompt","role":"user","text":"Original","sentAt":1})];
    let mut a = Store::open_monocode(&path).unwrap();
    insert_session(&a.conn, original, &blocks);
    let id = a.resolve_session_id(original).unwrap();
    if native_before {
        a.convert_session_to_chunked(id).unwrap();
    }
    let mut stale_rows = a.load(id).unwrap().rows;
    stale_rows[0].text = "Stale edit".into();
    let mut stale_blocks = a.stored_blocks(id).unwrap();
    stale_blocks[0]["text"] = json!("Stale edit");

    let mut b = Store::open_monocode(&path).unwrap();
    let other = b.resolve_session_id(original).unwrap();
    if native_before {
        b.downgrade_session_to_blob(other).unwrap();
    } else {
        b.convert_session_to_chunked(other).unwrap();
    }
    let mut latest = b.load(other).unwrap().rows;
    latest[0].text = "Other window committed".into();
    b.write_rows(other, 0, &latest).unwrap();

    let result = if full_replace {
        a.store_blocks(id, &stale_blocks)
    } else {
        a.write_rows(id, 0, &stale_rows)
    };
    assert!(
        result.is_err(),
        "stale writes must refuse after a mode flip"
    );
    assert_eq!(b.load(other).unwrap().rows, latest);
    // Reload explicitly acknowledges the new representation and permits saving.
    let mut refreshed = a.load(id).unwrap().rows;
    assert_eq!(refreshed, latest);
    refreshed[0].text = "Edit after reload".into();
    a.write_rows(id, 0, &refreshed).unwrap();
    assert_eq!(b.load(other).unwrap().rows, refreshed);
}

#[test]
fn review_blob_to_native_refuses_stale_sparse_write() {
    mode_flip_refuses_stale_write(false, false);
}

#[test]
fn review_native_to_blob_refuses_stale_sparse_write() {
    mode_flip_refuses_stale_write(true, false);
}

#[test]
fn review_blob_to_native_refuses_stale_full_replace() {
    mode_flip_refuses_stale_write(false, true);
}

#[test]
fn review_native_to_blob_refuses_stale_full_replace() {
    mode_flip_refuses_stale_write(true, true);
}

#[test]
fn review_conversion_seeds_committed_revision_without_reload() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    insert_session(
        &store.conn,
        "convert-and-save",
        &[json!({"id":"p","role":"user","text":"Before conversion","sentAt":1})],
    );
    let id = store.resolve_session_id("convert-and-save").unwrap();
    let mut rows = store.load(id).unwrap().rows;
    store.convert_session_to_chunked(id).unwrap();
    rows[0].text = "Saved immediately after conversion".into();
    store.write_rows(id, 0, &rows).unwrap();
    assert_eq!(store.load(id).unwrap().rows, rows);
}

#[test]
fn review_round_trip_mode_flip_refuses_stale_native_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mode-round-trip.db");
    let mut a = Store::open_monocode(&path).unwrap();
    insert_session(
        &a.conn,
        "round-trip",
        &[json!({"id":"p","role":"user","text":"Original","sentAt":1})],
    );
    let id = a.resolve_session_id("round-trip").unwrap();
    a.convert_session_to_chunked(id).unwrap();
    let mut stale = a.load(id).unwrap().rows;
    stale[0].text = "Stale edit".into();
    let mut b = Store::open_monocode(&path).unwrap();
    let other = b.resolve_session_id("round-trip").unwrap();
    b.downgrade_session_to_blob(other).unwrap();
    let mut latest = b.load(other).unwrap().rows;
    latest[0].text = "Committed between conversions".into();
    b.write_rows(other, 0, &latest).unwrap();
    b.convert_session_to_chunked(other).unwrap();
    assert!(
        a.write_rows(id, 0, &stale).is_err(),
        "reconversion must not reuse the stale revision"
    );
    assert_eq!(b.load(other).unwrap().rows, latest);
}

fn busy_commit_rolls_back(downgrade: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("busy-commit.db");
    let mut store = Store::open_monocode(&path).unwrap();
    seed_native_session(
        &store.conn,
        "busy",
        &[json!({"id":"p","role":"user","text":"Before","sentAt":1})],
    );
    let id = store.resolve_session_id("busy").unwrap();
    let mut rows = store.load(id).unwrap().rows;
    store
        .conn
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    store.conn.busy_timeout(Duration::ZERO).unwrap();
    let reader = Connection::open(&path).unwrap();
    reader.execute_batch("BEGIN").unwrap();
    let _: String = reader
        .query_row(
            "SELECT blocks_json FROM sessions WHERE id='busy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    rows[0].text = "After".into();
    let result = if downgrade {
        store.downgrade_session_to_blob(id)
    } else {
        store.write_rows(id, 0, &rows)
    };
    assert!(result.is_err(), "the held reader must make COMMIT busy");
    assert!(
        store.conn.is_autocommit(),
        "a failed commit must roll back its transaction"
    );
    reader.execute_batch("COMMIT").unwrap();
    assert!(store.storage_mode_version(id).unwrap().is_some());
    if downgrade {
        store.downgrade_session_to_blob(id).unwrap();
        assert!(store.storage_mode_version(id).unwrap().is_none());
    } else {
        store.write_rows(id, 0, &rows).unwrap();
        assert_eq!(store.load(id).unwrap().rows, rows);
    }
}

#[test]
fn review_sparse_busy_commit_rolls_back_and_can_retry() {
    busy_commit_rolls_back(false);
}

#[test]
fn review_downgrade_busy_commit_rolls_back_and_can_retry() {
    busy_commit_rolls_back(true);
}

#[test]
fn review_conversion_refuses_a_stale_loaded_blob() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stale-conversion.db");
    let mut a = Store::open_monocode(&path).unwrap();
    insert_session(
        &a.conn,
        "stale",
        &[json!({"id":"p","role":"user","text":"Original","sentAt":1})],
    );
    let id = a.resolve_session_id("stale").unwrap();
    a.load(id).unwrap();
    let mut b = Store::open_monocode(&path).unwrap();
    let other = b.resolve_session_id("stale").unwrap();
    let mut latest = b.load(other).unwrap().rows;
    latest[0].text = "Other window committed".into();
    b.write_rows(other, 0, &latest).unwrap();
    assert!(
        a.convert_session_to_chunked(id).is_err(),
        "conversion must not acknowledge an unseen edit"
    );
    assert_eq!(b.load(other).unwrap().rows, latest);
}

#[test]
fn review_downgrade_keeps_a_blob_guard_for_the_same_handle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("downgrade-guard.db");
    let mut a = Store::open_monocode(&path).unwrap();
    seed_native_session(
        &a.conn,
        "guard",
        &[json!({"id":"p","role":"user","text":"Original","sentAt":1})],
    );
    let id = a.resolve_session_id("guard").unwrap();
    let mut stale = a.load(id).unwrap().rows;
    a.downgrade_session_to_blob(id).unwrap();
    let mut b = Store::open_monocode(&path).unwrap();
    let other = b.resolve_session_id("guard").unwrap();
    let mut latest = b.load(other).unwrap().rows;
    latest[0].text = "Other window committed".into();
    b.write_rows(other, 0, &latest).unwrap();
    stale[0].text = "Stale edit".into();
    assert!(a.write_rows(id, 0, &stale).is_err());
    assert_eq!(b.load(other).unwrap().rows, latest);
}

fn stored_blocks_mode_flip_refuses_stale_replace(native_before: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blocks-guard.db");
    let mut a = Store::open_monocode(&path).unwrap();
    insert_session(
        &a.conn,
        "blocks",
        &[json!({"id":"p","role":"user","text":"Original","sentAt":1})],
    );
    let id = a.resolve_session_id("blocks").unwrap();
    if native_before {
        a.convert_session_to_chunked(id).unwrap();
    }
    drop(a);
    // A fresh handle has never loaded rows or acquired a revision guard.
    let a = Store::open_monocode(&path).unwrap();
    let id = a.resolve_session_id("blocks").unwrap();
    let mut stale = a.stored_blocks(id).unwrap();
    let mut b = Store::open_monocode(&path).unwrap();
    let other = b.resolve_session_id("blocks").unwrap();
    if native_before {
        b.downgrade_session_to_blob(other).unwrap();
    } else {
        b.convert_session_to_chunked(other).unwrap();
    }
    let mut latest = b.load(other).unwrap().rows;
    latest[0].text = "Other window committed".into();
    b.write_rows(other, 0, &latest).unwrap();
    stale[0]["text"] = json!("Stale edit");
    assert!(a.store_blocks(id, &stale).is_err());
    assert_eq!(b.load(other).unwrap().rows, latest);
}

#[test]
fn review_stored_blocks_guards_blob_to_native_replace() {
    stored_blocks_mode_flip_refuses_stale_replace(false);
}

#[test]
fn review_stored_blocks_guards_native_to_blob_replace() {
    stored_blocks_mode_flip_refuses_stale_replace(true);
}

#[test]
fn review_acceptance_of_the_only_draft_can_commit_and_retry() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let id = created_chunked(&store, "codex");
    store
        .store_blocks(
            id,
            &[json!({"id":"only-draft","role":"user","text":"Queued","draft":true})],
        )
        .unwrap();
    assert!(store.load(id).unwrap().rows.is_empty());
    let row = crate::transcript::Row {
        role: "user".into(),
        text: "Queued".into(),
        first: true,
        sent_at: Some(1),
    };
    store
        .write_rows_with_message(id, 0, std::slice::from_ref(&row), Some("only-draft"))
        .unwrap();
    assert_eq!(store.load(id).unwrap().rows, vec![row.clone()]);
    assert_eq!(
        store.stored_blocks(id).unwrap()[0]["appRequestId"],
        "only-draft"
    );
    store
        .write_rows_with_message(id, 0, std::slice::from_ref(&row), Some("only-draft"))
        .unwrap();
    assert_eq!(store.load(id).unwrap().rows, vec![row]);
}

#[test]
fn review_future_mode_refuses_full_history_and_page_reads() {
    let store = Store::open_monocode(Path::new(":memory:")).unwrap();
    seed_native_session(
        &store.conn,
        "future-read",
        &[json!({"id":"p","role":"user","text":"Synthetic","sentAt":1})],
    );
    store
        .conn
        .execute(
            "UPDATE native_sessions SET mode_version=?1 WHERE session_id='future-read'",
            [i64::from(NATIVE_CHUNKS_MODE_VERSION) + 1],
        )
        .unwrap();
    let id = store.resolve_session_id("future-read").unwrap();
    let error = store.load(id).unwrap_err();
    assert!(error.contains(UNSUPPORTED_STORAGE_MODE), "{error}");
    let error = store.stored_blocks(id).unwrap_err();
    assert!(error.contains(UNSUPPORTED_STORAGE_MODE), "{error}");
    let error = page_rows(&store.conn, "future-read", 0, 1).unwrap_err();
    assert!(error.contains(UNSUPPORTED_STORAGE_MODE), "{error}");
    let error = iter_full_history(&store.conn, "future-read")
        .collect::<Result<Vec<_>, _>>()
        .unwrap_err();
    assert!(error.contains(UNSUPPORTED_STORAGE_MODE), "{error}");
}

#[test]
fn review_public_store_blocks_rolls_back_and_can_retry() {
    let store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let session = created_chunked(&store, "codex");
    let original = store.persisted_session_id(session).unwrap();
    let before = vec![json!({"id":"p","role":"user","text":"Before","sentAt":1})];
    store.store_blocks(session, &before).unwrap();
    let before_rows = store.load(session).unwrap().rows;
    let snapshot = native_snapshot(&store.conn, &original);
    // The transcript update succeeds, then updating its list flags fails.
    store
        .conn
        .execute_batch(
            "CREATE TRIGGER refuse_list_flags BEFORE UPDATE OF has_user_message ON sessions
         BEGIN SELECT RAISE(ABORT, 'synthetic list flag failure'); END;",
        )
        .unwrap();
    let after = vec![json!({"id":"p","role":"user","text":"After","sentAt":1})];
    let error = store.store_blocks(session, &after).unwrap_err();
    assert!(error.contains("synthetic list flag failure"), "{error}");
    assert!(
        store.conn.is_autocommit(),
        "no transaction may leak on failure"
    );
    assert_eq!(native_snapshot(&store.conn, &original), snapshot);
    // Do not reload: a rolled-back save must be retryable with the same guard.
    store
        .conn
        .execute_batch("DROP TRIGGER refuse_list_flags")
        .unwrap();
    store.store_blocks(session, &after).unwrap();
    let reloaded = store.load(session).unwrap().rows;
    assert_ne!(reloaded, before_rows);
    assert_eq!(reloaded[0].text, "After");
}

#[test]
fn review_public_store_blocks_commit_failure_can_retry() {
    let store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let session = created_chunked(&store, "codex");
    let original = store.persisted_session_id(session).unwrap();
    let before = vec![json!({"id":"p","role":"user","text":"Before","sentAt":1})];
    store.store_blocks(session, &before).unwrap();
    store.load(session).unwrap();
    let snapshot = native_snapshot(&store.conn, &original);
    store.fail_next_save_commit_for_test().unwrap();
    let after = vec![json!({"id":"p","role":"user","text":"After %NATIVE_ROWS%","sentAt":1})];
    assert!(store.store_blocks(session, &after).is_err());
    assert!(store.conn.is_autocommit());
    assert_eq!(native_snapshot(&store.conn, &original), snapshot);
    assert_eq!(store.storage_mode_version(session).unwrap(), Some(1));
    store.store_blocks(session, &after).unwrap();
    assert_eq!(store.storage_mode_version(session).unwrap(), Some(2));
    assert_eq!(
        store.load(session).unwrap().rows[0].text,
        "After %NATIVE_ROWS%"
    );
}

#[test]
fn review_public_store_blocks_marker_upgrade_can_save_again_without_reopen() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let session = created_chunked(&store, "codex");
    let before = vec![json!({"id":"p","role":"user","text":"Before","sentAt":1})];
    store.store_blocks(session, &before).unwrap();
    assert_eq!(store.storage_mode_version(session).unwrap(), Some(1));

    let after = vec![json!({"id":"p","role":"user","text":"After %NATIVE_ROWS%","sentAt":1})];
    store.store_blocks(session, &after).unwrap();
    assert_eq!(store.storage_mode_version(session).unwrap(), Some(2));

    store
        .write_rows(session, 1, &[appended_row("Follow-up")])
        .unwrap();
    let rows = store.load(session).unwrap().rows;
    assert_eq!(rows[0].text, "After %NATIVE_ROWS%");
    assert_eq!(rows[1].text, "Follow-up");
}

#[test]
fn review_receipt_skips_deleted_draft_when_updating_existing_prompt() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let session = created_chunked(&store, "codex");
    store
        .store_blocks(
            session,
            &[
                json!({"id":"prompt","role":"user","text":"Existing prompt","sentAt":1}),
                json!({"id":"queued","role":"user","text":"Queued prompt","draft":true}),
            ],
        )
        .unwrap();
    let original = store.persisted_session_id(session).unwrap();
    store.conn.execute(
        "UPDATE sessions SET queued_messages_json='[{\"id\":\"queued\",\"text\":\"Queued prompt\"}]' WHERE id=?1",
        [&original],
    ).unwrap();
    let rows = store.load(session).unwrap().rows;
    store
        .write_rows_with_message(session, 0, &rows, Some("queued"))
        .unwrap();
    let saved = store.stored_blocks(session).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0]["appRequestId"], "queued");
    let queued: String = store
        .conn
        .query_row(
            "SELECT queued_messages_json FROM sessions WHERE id=?1",
            [&original],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(queued, "[]");
    assert!(!store.draft_sessions().unwrap().contains(&session));
}

#[test]
fn existing_blob_sessions_untouched() {
    let blocks = vec![
        json!({"id":"p","role":"user","text":"Write a report","sentAt":123,"futureMetadata":{"keep":[1,2]}}),
        json!({"id":"r","role":"assistant","text":"Done","futureMetadata":true}),
    ];
    // Two identically seeded databases: one with the batch-2 tables a new
    // database now gains, one without them, as every pre-batch-2 database is.
    let with_tables = tempfile::tempdir().unwrap();
    let legacy = tempfile::tempdir().unwrap();
    let mut saved = Vec::new();
    for (dir, drop_tables) in [(&with_tables, false), (&legacy, true)] {
        let path = dir.path().join("blob.db");
        {
            let store = Store::open_monocode(&path).unwrap();
            insert_session(&store.conn, "blob-1", &blocks);
            if drop_tables {
                store
                    .conn
                    .execute_batch(
                        "DROP TABLE native_chunks; DROP TABLE native_blocks; DROP TABLE native_sessions;",
                    )
                    .unwrap();
            }
        }
        let mut store = Store::open_monocode(&path).unwrap();
        // An absent marker is Blob, with or without the marker table at all.
        assert_eq!(
            session_storage_mode(&store.conn, "blob-1").unwrap(),
            StorageMode::Blob
        );
        let id = store.resolve_session_id("blob-1").unwrap();
        let mut transcript = store.load(id).unwrap();
        // A deterministic in-place edit of the existing reply: unlike appending,
        // it mints no new block identity or send time, so both databases must
        // store byte-identical bytes.
        transcript.rows[1].text = "Done and checked 🙂".into();
        store.write_rows(id, 1, &transcript.rows[1..]).unwrap();
        let raw: String = store
            .conn
            .query_row(
                "SELECT blocks_json FROM sessions WHERE id='blob-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        if !drop_tables {
            // The blob path never touches the chunked backend's tables.
            let leaked: i64 = store
                .conn
                .query_row(
                    "SELECT (SELECT count(*) FROM native_sessions)
                           + (SELECT count(*) FROM native_blocks)
                           + (SELECT count(*) FROM native_chunks)",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(leaked, 0);
        }
        saved.push((raw, transcript));
    }
    // The blob write is byte-identical whether or not the chunk tables exist,
    // really landed, and kept the unknown fields.
    assert_eq!(saved[0].0, saved[1].0);
    assert_ne!(saved[0].0, serde_json::to_string(&blocks).unwrap());
    assert!(saved[0].0.contains("Done and checked 🙂"));
    assert!(saved[0].0.contains("futureMetadata"));
    // Reload from the database that has the tables: rows and stored bytes hold.
    let path = with_tables.path().join("blob.db");
    let store = Store::open_monocode(&path).unwrap();
    let id = store.resolve_session_id("blob-1").unwrap();
    assert_eq!(store.load(id).unwrap().rows, saved[0].1.rows);
    let raw: String = store
        .conn
        .query_row(
            "SELECT blocks_json FROM sessions WHERE id='blob-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(raw, saved[0].0);
}

// ---- Task 4: the sparse block-level writer ----

/// Seed a chunked-mode session: a sessions row, the mode marker and one
/// skeleton+chunks block per value, laid out exactly as the writer stores
/// them. `blocks_json` stays empty — the chunked backend owns the transcript.
pub(crate) fn seed_native_session(conn: &Connection, id: &str, blocks: &[Value]) {
    insert_session(conn, id, &[]);
    conn.execute(
        "INSERT INTO native_sessions(session_id,mode_version,session_revision) VALUES(?1,?2,0)",
        params![id, NATIVE_CHUNKS_MODE_VERSION as i64],
    )
    .unwrap();
    seed_native_blocks(conn, id, blocks);
}

/// Store block values as native_blocks/native_chunks/native_payloads rows in
/// block order, laid out exactly as the writer stores them: the canonical
/// stored packing for the row elements, segmented payload members for whatever
/// `store_block_form` stripped.
pub(crate) fn seed_native_blocks(conn: &Connection, id: &str, blocks: &[Value]) {
    let mut first_row = 0_i64;
    for (seq, block) in blocks.iter().enumerate() {
        let raw = serde_json::to_string(block).unwrap();
        let rows = blocks::project(std::slice::from_ref(block)).unwrap();
        let form = store_block_form(block, &raw, &rows).unwrap();
        let chunks = pack_stored_chunks(&form.elements, CHUNK_TARGET_BYTES);
        conn.execute(
            "INSERT INTO native_blocks(session_id,block_seq,block_id,skeleton_json,first_row,row_count,block_revision)
             VALUES(?1,?2,?3,?4,?5,?6,0)",
            params![id, seq as i64, block["id"].as_str(), form.skeleton, first_row, rows.len() as i64],
        )
        .unwrap();
        for (ord, chunk) in chunks.iter().enumerate() {
            conn.execute(
                "INSERT INTO native_chunks(session_id,block_seq,chunk_ord,rows_json,byte_len)
                 VALUES(?1,?2,?3,?4,?5)",
                params![id, seq as i64, ord as i64, chunk, chunk.len() as i64],
            )
            .unwrap();
        }
        for (member, payload) in form.payloads.iter().enumerate() {
            for (seg, piece) in pack_payload_segments(payload, CHUNK_TARGET_BYTES)
                .iter()
                .enumerate()
            {
                conn.execute(
                    "INSERT INTO native_payloads(session_id,block_seq,member,seg,payload_json,byte_len)
                     VALUES(?1,?2,?3,?4,?5,?6)",
                    params![id, seq as i64, member as i64, seg as i64, piece, piece.len() as i64],
                )
                .unwrap();
            }
        }
        first_row += rows.len() as i64;
    }
}

/// Everything a write must not disturb: per block its sequence, skeleton,
/// chunks and revision, then the session revision, then its stored payload
/// segments by member.
pub(crate) type NativeSnapshot = Vec<(i64, String, Vec<String>, i64, i64, Vec<Vec<String>>)>;
pub(crate) fn native_snapshot(conn: &Connection, id: &str) -> NativeSnapshot {
    let mut blocks = conn
        .prepare("SELECT block_seq,skeleton_json,block_revision FROM native_blocks WHERE session_id=?1 ORDER BY block_seq")
        .unwrap();
    let stored = blocks
        .query_map(params![id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    drop(blocks);
    let revision: i64 = conn
        .query_row(
            "SELECT session_revision FROM native_sessions WHERE session_id=?1",
            params![id],
            |row| row.get(0),
        )
        .unwrap();
    let mut out = Vec::new();
    for (seq, skeleton, block_revision) in stored {
        let mut chunks = conn
            .prepare("SELECT rows_json FROM native_chunks WHERE session_id=?1 AND block_seq=?2 ORDER BY chunk_ord")
            .unwrap();
        let rows = chunks
            .query_map(params![id, seq], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        drop(chunks);
        let mut payloads = conn
            .prepare("SELECT member,payload_json FROM native_payloads WHERE session_id=?1 AND block_seq=?2 ORDER BY member,seg")
            .unwrap();
        let segments = payloads
            .query_map(params![id, seq], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        drop(payloads);
        let mut members: Vec<Vec<String>> = Vec::new();
        for (member, json) in segments {
            if members.len() <= member as usize {
                members.resize(member as usize + 1, Vec::new());
            }
            members[member as usize].push(json);
        }
        out.push((seq, skeleton, rows, block_revision, revision, members));
    }
    out
}

/// The session's rows, reconstructed from the stored form in block order.
pub(crate) fn native_rows(conn: &Connection, id: &str) -> Vec<crate::transcript::Row> {
    let mut out = Vec::new();
    let mut blocks = conn
        .prepare("SELECT block_seq FROM native_blocks WHERE session_id=?1 ORDER BY block_seq")
        .unwrap();
    let stored = blocks
        .query_map(params![id], |row| row.get::<_, i64>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    drop(blocks);
    for seq in stored {
        let (skeleton, chunks, payloads) = stored_payload(conn, id, seq).unwrap();
        // `block_value`, not the raw splice: a stripped `text` must come back
        // from the rows before the block is projected.
        let value = block_value(&skeleton, &chunks, &payloads).unwrap();
        out.extend(blocks::project(std::slice::from_ref(&value)).unwrap());
    }
    out
}

/// One stored block's payload size in bytes: skeleton plus chunks.
fn payload_bytes(conn: &Connection, id: &str, block_seq: i64) -> u64 {
    let skeleton: String = conn
        .query_row(
            "SELECT skeleton_json FROM native_blocks WHERE session_id=?1 AND block_seq=?2",
            params![id, block_seq],
            |row| row.get(0),
        )
        .unwrap();
    let chunks: i64 = conn
        .query_row(
            "SELECT coalesce(sum(byte_len),0) FROM native_chunks WHERE session_id=?1 AND block_seq=?2",
            params![id, block_seq],
            |row| row.get(0),
        )
        .unwrap();
    skeleton.len() as u64 + chunks.max(0) as u64
}

/// Render a block value exactly as the sparse writer's callers do.
fn rendered(block: &Value) -> BlockUpdate {
    let raw = serde_json::to_string(block).unwrap();
    let rows = blocks::project(std::slice::from_ref(block)).unwrap();
    let form = store_block_form(block, &raw, &rows).unwrap();
    BlockUpdate {
        skeleton: form.skeleton,
        rows: form.elements,
        payloads: form.payloads,
    }
}

#[test]
fn sparse_flush_touches_only_dirty_blocks() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let blocks = vec![
        json!({"id":"prompt","role":"user","text":"Write a report","sentAt":1,"future":"keep"}),
        json!({"id":"run","role":"tool","text":"Run checks","sentAt":2,
            "tool":{"callId":"call-1","nativeName":"Bash","nativeInput":"{\"command\":\"make check\"}",
                    "kind":"execute","title":"make check","status":"completed","futureToolField":[1]}}),
        json!({"id":"middle","role":"assistant","text":"Middle reply","sentAt":3}),
        json!({"id":"tail","role":"assistant","text":"Tail reply","sentAt":4}),
    ];
    let id = "sparse-dirty";
    seed_native_session(&store.conn, id, &blocks);
    let before = native_snapshot(&store.conn, id);
    // Dirty only the early tool block: its detail grows in place, same row count.
    let mut edited = blocks[1].clone();
    edited["tool"]["detail"] = json!("lint: 3 findings 🙂");
    let update = rendered(&edited);
    let plan = SparseSavePlan {
        session_revision_expected: 0,
        dirty: vec![DirtyBlock {
            block_seq: 1,
            first_row: 1,
            last_row: 1,
        }],
    };
    let flush = flush_sparse(&mut store.conn, id, &plan, &|_| Ok(update.clone())).unwrap();
    assert_eq!(flush.session_revision, 1);
    assert_eq!(flush.blocks_written, 1);
    let after = native_snapshot(&store.conn, id);
    // Untouched blocks keep every byte, their revision and the row layout; the
    // session revision moves for the whole session, so it is asserted apart.
    for (was, now) in before.iter().zip(&after) {
        if was.0 == 1 {
            continue;
        }
        assert_eq!(
            (&was.1, &was.2, was.3),
            (&now.1, &now.2, now.3),
            "block {} must be untouched",
            was.0
        );
    }
    // The dirty block changed, its revision moved, and only its chunks were written.
    assert_eq!(after[1].3, 1);
    assert_ne!(after[1].2, before[1].2);
    assert_eq!(flush.chunks_written, after[1].2.len() as u64);
    assert!(flush.chunk_bytes_written > 0);
    let raw = block_raw_json(&after[1].1, &after[1].2, &[]).unwrap();
    assert!(raw.contains("lint: 3 findings 🙂"));
    assert!(raw.contains("futureToolField"));
    let value: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        blocks::project(std::slice::from_ref(&value)).unwrap().len(),
        1
    );
    assert_eq!(native_rows(&store.conn, id).len(), 4);
}

#[test]
fn early_tool_plus_tail_token_one_commit() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let filler = || "完成的段落 🙂 café 漢字".repeat(320);
    // A tail exactly one row wide: the next token crosses the row-width
    // boundary, so the streamed tail really does grow a row.
    let tail_text = "W".repeat(crate::transcript::ROW_BYTES);
    let blocks = vec![
        json!({"id":"tool-early","role":"tool","text":"make check","sentAt":1,
            "tool":{"callId":"c","nativeName":"Bash","nativeInput":"{\"command\":\"make check\"}",
                    "kind":"execute","title":"make check","status":"completed"}}),
        json!({"id":"mid-1","role":"assistant","text":filler(),"sentAt":2}),
        json!({"id":"mid-2","role":"assistant","text":filler(),"sentAt":3}),
        json!({"id":"mid-3","role":"assistant","text":filler(),"sentAt":4}),
        json!({"id":"tail","role":"assistant","text":tail_text,"sentAt":5}),
    ];
    let id = "early-tail";
    seed_native_session(&store.conn, id, &blocks);
    let handle = store.resolve_session_id(id).unwrap();
    let before = native_snapshot(&store.conn, id);
    // The payload bytes the two dirty blocks occupy before the save; the save
    // itself rewrites block 0 and grows the tail, so this is measured first.
    let dirty_bytes = payload_bytes(&store.conn, id, 0) + payload_bytes(&store.conn, id, 4);
    let mut rows = blocks::project(&blocks).unwrap();
    // An early in-place tool status edit plus one appended tail token: two
    // non-adjacent ranges, one save. The edit goes through the tool row's own
    // type so the expected text keeps the projection's field order, and the
    // tail rows are streamed exactly as the runtime streams them, so
    // re-projecting the merged block reproduces them.
    let mut call: crate::tool::ToolCall = serde_json::from_str(&rows[0].text).unwrap();
    call.output = "early status edit".into();
    rows[0].text = serde_json::to_string(&call).unwrap();
    let early = [rows[0].clone()];
    let mut stream = crate::transcript::Transcript::default();
    stream.begin("assistant");
    stream.append(&tail_text);
    stream.append(" and a tail token 中文");
    let mut tail = stream.rows;
    tail[0].sent_at = rows.last().unwrap().sent_at;
    let tail_start = rows.len() - 1;
    rows.truncate(tail_start);
    store
        .write_row_ranges(handle, &[(0, &early), (tail_start, &tail)])
        .unwrap();
    let sample = store.take_save_sample().unwrap();
    // One transaction for both ranges.
    assert_eq!(sample.commit_count, 1);
    // Middle blocks never entered the write set: only the dirty payloads were read.
    assert_eq!(sample.history_bytes_read, dirty_bytes);
    assert!(
        dirty_bytes * 3 < payload_bytes(&store.conn, id, 1),
        "the read set must stay far below one middle block"
    );
    let after = native_snapshot(&store.conn, id);
    for seq in 1..=3 {
        assert_eq!(
            (&before[seq].1, &before[seq].2, before[seq].3),
            (&after[seq].1, &after[seq].2, after[seq].3),
            "middle block {seq} must be untouched"
        );
    }
    assert_eq!(after[0].3, 1);
    assert_eq!(after[4].3, 1);
    assert_eq!(after[0].4, 1);
    // The token extended the tail block, not minted a new one.
    assert_eq!(after.len(), 5);
    // The token extended the tail block, not minted a new one. (The skeleton's
    // rows placeholder carries a W of its own, so count the spliced block.)
    let spliced_tail = block_raw_json(&after[4].1, &after[4].2, &[]).unwrap();
    let spliced_tail: Value = serde_json::from_str(&spliced_tail).unwrap();
    assert_eq!(
        spliced_tail["text"].as_str().unwrap().matches('W').count(),
        crate::transcript::ROW_BYTES
    );
    assert!(spliced_tail["text"]
        .as_str()
        .unwrap()
        .ends_with(" and a tail token 中文"));
    rows.extend(tail);
    assert_eq!(native_rows(&store.conn, id), rows);
}

#[test]
fn stale_write_rejected_other_side_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stale.db");
    let blocks = vec![
        json!({"id":"shared-prompt","role":"user","text":"Original","sentAt":1}),
        json!({"id":"shared-reply","role":"assistant","text":"Reply","sentAt":2}),
    ];
    let id = "contended";
    let mut a = Store::open_monocode(&path).unwrap();
    seed_native_session(&a.conn, id, &blocks);
    // Window B: another connection on the same database commits first.
    let mut b = Connection::open(&path).unwrap();
    b.busy_timeout(Duration::from_secs(5)).unwrap();
    let mut b_edit = blocks[0].clone();
    b_edit["text"] = json!("B edited 🙂");
    let b_update = rendered(&b_edit);
    let plan_b = SparseSavePlan {
        session_revision_expected: 0,
        dirty: vec![DirtyBlock {
            block_seq: 0,
            first_row: 0,
            last_row: 0,
        }],
    };
    let flushed = flush_sparse(&mut b, id, &plan_b, &|_| Ok(b_update.clone())).unwrap();
    assert_eq!(flushed.session_revision, 1);
    // Window A flushes with the stale expected revision: refused, never merged.
    let mut a_edit = blocks[0].clone();
    a_edit["text"] = json!("A edited");
    let a_update = rendered(&a_edit);
    let plan_a = SparseSavePlan {
        session_revision_expected: 0,
        dirty: vec![DirtyBlock {
            block_seq: 0,
            first_row: 0,
            last_row: 0,
        }],
    };
    let error = flush_sparse(&mut a.conn, id, &plan_a, &|_| Ok(a_update.clone())).unwrap_err();
    assert!(
        matches!(error, FlushError::Conflict(FlushConflict::StaleSession)),
        "{error}"
    );
    // B's committed content survives A's refusal untouched.
    let snapshot = native_snapshot(&a.conn, id);
    assert_eq!(snapshot.len(), 2);
    assert_eq!(snapshot[0].3, 1);
    assert_eq!(snapshot[0].4, 1);
    let raw = block_raw_json(&snapshot[0].1, &snapshot[0].2, &[]).unwrap();
    assert!(raw.contains("B edited"));
    assert!(!raw.contains("A edited"));
    // A re-read session state but a block moved underneath: refused per block.
    a.conn
        .execute(
            "UPDATE native_blocks SET first_row=first_row+1 WHERE session_id=?1 AND block_seq=1",
            params![id],
        )
        .unwrap();
    let plan_block = SparseSavePlan {
        session_revision_expected: 1,
        dirty: vec![DirtyBlock {
            block_seq: 1,
            first_row: 1,
            last_row: 1,
        }],
    };
    let error =
        flush_sparse(&mut a.conn, id, &plan_block, &|_| Ok(rendered(&blocks[1]))).unwrap_err();
    assert!(
        matches!(
            error,
            FlushError::Conflict(FlushConflict::StaleBlock { block_seq: 1 })
        ),
        "{error}"
    );
    // Neither refusal wrote anything.
    let after = native_snapshot(&a.conn, id);
    assert_eq!(after[0].4, 1);
    assert_eq!(after[1].3, snapshot[1].3);
}

#[test]
fn zero_dirty_no_write() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let blocks = vec![
        json!({"id":"p","role":"user","text":"Prompt","sentAt":1}),
        json!({"id":"r","role":"assistant","text":"Reply","sentAt":2}),
    ];
    let id = "zero-dirty";
    seed_native_session(&store.conn, id, &blocks);
    let handle = store.resolve_session_id(id).unwrap();
    let rows = vec![
        crate::transcript::Row {
            role: "user".into(),
            text: "Next".into(),
            first: true,
            sent_at: Some(3),
        },
        crate::transcript::Row {
            role: "assistant".into(),
            text: String::new(),
            first: true,
            sent_at: None,
        },
    ];
    store
        .write_rows_with_message(handle, 2, &rows, Some("queued-1"))
        .unwrap();
    let after_first = native_snapshot(&store.conn, id);
    assert_eq!(after_first.len(), 4);
    assert_eq!(after_first[0].4, 1);
    // Accepting the very same rows and message again writes no transcript
    // bytes: every byte, revision and counter stays put (same-value
    // acceptance must not touch the transcript).
    store
        .write_rows_with_message(handle, 2, &rows, Some("queued-1"))
        .unwrap();
    assert_eq!(native_snapshot(&store.conn, id), after_first);
    // A literally empty plan touches nothing and reports the current revision.
    let flush = flush_sparse(
        &mut store.conn,
        id,
        &SparseSavePlan {
            session_revision_expected: 1,
            dirty: Vec::new(),
        },
        &|_| panic!("an empty plan renders nothing"),
    )
    .unwrap();
    assert_eq!(
        flush,
        SparseFlush {
            session_revision: 1,
            mode_version: NATIVE_CHUNKS_MODE_VERSION,
            blocks_written: 0,
            chunks_written: 0,
            chunk_bytes_written: 0,
            skeleton_bytes_written: 0,
        }
    );
}

#[test]
fn duplicate_or_missing_block_id_refused() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let blocks = vec![json!({"id":"only","role":"user","text":"Prompt","sentAt":1})];
    let id = "refused";
    seed_native_session(&store.conn, id, &blocks);
    let baseline = native_snapshot(&store.conn, id);
    // A plan referencing an unknown block_seq — neither stored nor the next
    // append position — is refused with a distinct error, never dropped.
    let stray = SparseSavePlan {
        session_revision_expected: 0,
        dirty: vec![DirtyBlock {
            block_seq: 7,
            first_row: 1,
            last_row: 1,
        }],
    };
    let error = flush_sparse(&mut store.conn, id, &stray, &|_| {
        Ok(rendered(&json!({"id":"stray","role":"user","text":"x"})))
    })
    .unwrap_err();
    assert!(
        matches!(error, FlushError::UnknownBlock { block_seq: 7 }),
        "{error}"
    );
    assert!(error.to_string().contains("block_seq 7"));
    // The right sequence but the wrong row position is still an unknown block.
    let misplaced = SparseSavePlan {
        session_revision_expected: 0,
        dirty: vec![DirtyBlock {
            block_seq: 1,
            first_row: 5,
            last_row: 5,
        }],
    };
    let error = flush_sparse(&mut store.conn, id, &misplaced, &|_| {
        Ok(rendered(&json!({"id":"new","role":"user","text":"x"})))
    })
    .unwrap_err();
    assert!(
        matches!(error, FlushError::UnknownBlock { block_seq: 1 }),
        "{error}"
    );
    // Two appended blocks claiming one block id.
    let duplicate = SparseSavePlan {
        session_revision_expected: 0,
        dirty: vec![
            DirtyBlock {
                block_seq: 1,
                first_row: 1,
                last_row: 1,
            },
            DirtyBlock {
                block_seq: 2,
                first_row: 2,
                last_row: 2,
            },
        ],
    };
    let render = |block: &DirtyBlock| {
        Ok(rendered(
            &json!({"id":"dup","role":"assistant","text":block.block_seq.to_string()}),
        ))
    };
    let error = flush_sparse(&mut store.conn, id, &duplicate, &render).unwrap_err();
    assert!(
        matches!(error, FlushError::DuplicateBlockId { ref block_id } if block_id == "dup"),
        "{error}"
    );
    assert!(error.to_string().contains("dup"));
    // An appended block without an id.
    let missing = SparseSavePlan {
        session_revision_expected: 0,
        dirty: vec![DirtyBlock {
            block_seq: 1,
            first_row: 1,
            last_row: 1,
        }],
    };
    let error = flush_sparse(&mut store.conn, id, &missing, &|_| {
        Ok(rendered(&json!({"role":"assistant","text":"no id"})))
    })
    .unwrap_err();
    assert!(
        matches!(error, FlushError::MissingBlockId { block_seq: 1 }),
        "{error}"
    );
    // The same stored block twice in one plan.
    let twice = SparseSavePlan {
        session_revision_expected: 0,
        dirty: vec![
            DirtyBlock {
                block_seq: 0,
                first_row: 0,
                last_row: 0,
            },
            DirtyBlock {
                block_seq: 0,
                first_row: 0,
                last_row: 0,
            },
        ],
    };
    let error =
        flush_sparse(&mut store.conn, id, &twice, &|_| Ok(rendered(&blocks[0]))).unwrap_err();
    assert!(
        matches!(error, FlushError::DuplicateBlock { block_seq: 0 }),
        "{error}"
    );
    // No refusal wrote anything.
    assert_eq!(native_snapshot(&store.conn, id), baseline);
}

// ---- Task 5: intra-block chunking ----

/// One growing assistant block of at least 5 MiB: the projection splits its
/// text into ~2 KiB rows, which the canonical packing turns into ~64 KiB
/// chunks, so a bounded save's write set can be told apart from the block.
fn growing_block(text: &str) -> Value {
    json!({"id":"growing","role":"assistant","text":text,"sentAt":1})
}

fn large_block_text() -> String {
    format!("{}{}", "0123456789abcdef", "a".repeat(5 * 1024 * 1024))
}

#[test]
fn append_rewrites_only_tail_chunk() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let id = "tail-chunk";
    seed_native_session(&store.conn, id, &[growing_block(&large_block_text())]);
    let handle = store.resolve_session_id(id).unwrap();
    let before = native_snapshot(&store.conn, id);
    let chunk_bytes: u64 = before[0].2.iter().map(|chunk| chunk.len() as u64).sum();
    let mut rows =
        blocks::project(std::slice::from_ref(&growing_block(&large_block_text()))).unwrap();
    // The synthetic block is genuinely large: ≥5 MiB of rows across many chunks.
    assert!(chunk_bytes >= 5 * 1024 * 1024);
    assert!(before[0].2.len() > 8);
    for round in 0..4 {
        let mut text = large_block_text();
        text.push_str(&"x".repeat(1024 * (round + 1)));
        let grown = growing_block(&text);
        let new_rows = blocks::project(std::slice::from_ref(&grown)).unwrap();
        // Appends only ever touch the tail rows; earlier rows are append-stable.
        assert_eq!(new_rows[..rows.len() - 1], rows[..rows.len() - 1]);
        store
            .write_row_ranges(handle, &[(rows.len() - 1, &new_rows[rows.len() - 1..])])
            .unwrap();
        let sample = store.take_save_sample().expect("save sample");
        assert_eq!(sample.blocks_written, 1, "round {round}");
        assert!(
            sample.chunks_written <= 2,
            "round {round}: {} chunks rewritten",
            sample.chunks_written
        );
        // ~chunk target + the appended 1 KiB (+ one row's envelope): never the
        // 5 MiB block's size.
        assert!(
            sample.chunk_bytes_written <= (CHUNK_TARGET_BYTES + 1024 + 2048) as u64,
            "round {round}: {} bytes written",
            sample.chunk_bytes_written
        );
        assert!(sample.chunk_bytes_written >= 1024, "round {round}");
        assert!(
            sample.chunk_bytes_written * 32 < chunk_bytes,
            "round {round}: the write must not scale with the block"
        );
        let after = native_snapshot(&store.conn, id);
        // At most the tail chunk was rewritten and one chunk appended: every
        // earlier chunk keeps its exact stored bytes.
        assert_eq!(
            after[0].2[..before[0].2.len() - 1],
            before[0].2[..before[0].2.len() - 1],
            "round {round}"
        );
        assert_eq!(native_rows(&store.conn, id), new_rows, "round {round}");
        rows = new_rows;
    }
}

#[test]
fn early_row_edit_rewrites_only_its_chunk() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let id = "early-chunk";
    seed_native_session(&store.conn, id, &[growing_block(&large_block_text())]);
    let handle = store.resolve_session_id(id).unwrap();
    let before = native_snapshot(&store.conn, id);
    // A size-stable edit of the first row: its serialized width — and with it
    // every later row and chunk boundary — stays fixed, so only the chunk
    // holding the row can move.
    let mut text = large_block_text();
    text.replace_range(0..16, "fedcba9876543210");
    let edited = growing_block(&text);
    let rows = blocks::project(std::slice::from_ref(&edited)).unwrap();
    // The edit is size-stable: same row count, only row 0 moved.
    let seeded =
        blocks::project(std::slice::from_ref(&growing_block(&large_block_text()))).unwrap();
    assert_eq!(rows.len(), seeded.len());
    assert_ne!(rows[0], seeded[0]);
    assert_eq!(rows[1..], seeded[1..]);
    store.write_row_ranges(handle, &[(0, &rows[..1])]).unwrap();
    let sample = store.take_save_sample().unwrap();
    assert_eq!(sample.blocks_written, 1);
    assert_eq!(sample.chunks_written, 1);
    let after = native_snapshot(&store.conn, id);
    assert_eq!(
        sample.chunk_bytes_written,
        after[0].2[0].len() as u64,
        "the counter is exactly the rewritten chunk's bytes"
    );
    assert!(sample.chunk_bytes_written <= (CHUNK_TARGET_BYTES + 512) as u64);
    // Tail chunks are byte-identical and untouched; the first chunk moved.
    assert_eq!(after[0].2[1..], before[0].2[1..]);
    assert_ne!(after[0].2[0], before[0].2[0]);
    assert_eq!(after[0].3, before[0].3 + 1);
    assert_eq!(native_rows(&store.conn, id), rows);
}

#[test]
fn chunk_rows_rewrites_only_the_moved_span() {
    use super::writer::{chunk_rows, ChunkOp};
    // Rows over half the chunk target, so the canonical packing holds exactly
    // one row per chunk and the planned ops stay readable.
    let row = |tag: char| {
        serde_json::to_string(&json!({"role":"assistant","text":tag.to_string().repeat(40 * 1024)}))
            .unwrap()
    };
    let ops = |existing: &[(i64, &str)], rows: &[String]| -> Vec<String> {
        chunk_rows(rows, existing)
            .into_iter()
            .map(|op| match op {
                ChunkOp::Rewrite(ord, _) => format!("rewrite:{ord}"),
                ChunkOp::Append(_) => "append".into(),
                ChunkOp::Truncate(keep) => format!("truncate:{keep}"),
            })
            .collect::<Vec<_>>()
    };
    let stored = |rows: &[String]| -> Vec<(i64, String)> {
        rows.iter()
            .enumerate()
            .map(|(ord, row)| (ord as i64, row.clone()))
            .collect()
    };
    fn refs(rows: &[(i64, String)]) -> Vec<(i64, &str)> {
        rows.iter().map(|(ord, row)| (*ord, row.as_str())).collect()
    }
    let a = [row('a'), row('b'), row('c')];
    let d = row('d');
    // A brand-new block appends every chunk; the same rows again write nothing.
    assert_eq!(ops(&[], &a), vec!["append", "append", "append"]);
    let base = stored(&a);
    assert!(ops(&refs(&base), &a).is_empty());
    // An edit that keeps the row's width moves only its chunk: the identical
    // chunks on both sides of the change are anchored and skipped.
    let edited_middle = [a[0].clone(), row('B'), a[2].clone()];
    assert_eq!(ops(&refs(&base), &edited_middle), vec!["rewrite:1"]);
    // A pure append adds the new chunk without touching the stored ones.
    let mut appended = a.to_vec();
    appended.push(d.clone());
    assert_eq!(ops(&refs(&base), &appended), vec!["append"]);
    // An edit plus an append rewrites the span between them.
    let both = [a[0].clone(), row('B'), a[2].clone(), d.clone()];
    assert_eq!(
        ops(&refs(&base), &both),
        vec!["rewrite:1", "rewrite:2", "append"]
    );
    // Fewer chunks than stored truncate the orphans from the first one gone.
    assert_eq!(ops(&refs(&base), &a[..1]), vec!["truncate:1"]);
}

// ---- Task 6: readers, absolute row numbers, giant-row bounded writes ----

/// A session created through the real opt-in flow: the store prefers chunked
/// storage, so `create` lands the marker before any transcript row.
fn created_chunked(store: &Store, provider: &str) -> i64 {
    store.prefer_chunked_storage(true).unwrap();
    store.create(provider, "/project").unwrap().id
}

fn stored_block_count(conn: &Connection, id: &str, table: &str) -> i64 {
    conn.query_row(
        &format!("SELECT count(*) FROM {table} WHERE session_id=?1"),
        params![id],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn giant_members_strip_and_split_with_markers() {
    // A row element larger than the target splits at character boundaries —
    // multibyte content never produces invalid UTF-8 pieces — and every piece
    // after the first carries the continuation marker.
    let element = serde_json::to_string(&json!({
        "role": "assistant",
        "text": "é漢🙂".repeat(40 * 1024)
    }))
    .unwrap();
    let pieces = pack_stored_chunks(std::slice::from_ref(&element), 64);
    assert!(pieces.len() > 3);
    assert!(pieces.iter().all(|piece| piece.len() <= 64));
    assert!(!pieces[0].starts_with(super::layout::CHUNK_CONTINUATION));
    assert!(pieces[1..]
        .iter()
        .all(|piece| piece.starts_with(super::layout::CHUNK_CONTINUATION)));

    // A giant `text` strips to the sentinel; the value comes back from the row
    // group join, so it round-trips through the stored form.
    let text = "é".repeat(CHUNK_TARGET_BYTES + 1);
    let block = json!({"id":"giant-text","role":"assistant","text":text,"sentAt":1,"future":true});
    let rows = blocks::project(std::slice::from_ref(&block)).unwrap();
    let raw = serde_json::to_string(&block).unwrap();
    let form = store_block_form(&block, &raw, &rows).unwrap();
    assert!(form.skeleton.contains(TEXT_SENTINEL));
    assert!(form.payloads.is_empty());
    assert_eq!(
        block_value(&form.skeleton, &form.elements, &form.payloads).unwrap(),
        block
    );

    // A giant tool member strips to a payload placeholder and its serialized
    // value is restored from the segments, in document order when there are
    // several.
    let input = "x".repeat(CHUNK_TARGET_BYTES + 1);
    let raw_input = "y".repeat(CHUNK_TARGET_BYTES + 2);
    let tool = json!({"id":"giant-tool","role":"tool","text":"Run","sentAt":1,
        "tool":{"callId":"c","nativeName":"Bash","nativeInput":input,"rawInput":raw_input,
                "kind":"execute","title":"Run","status":"running","future":[1]}});
    let rows = blocks::project(std::slice::from_ref(&tool)).unwrap();
    let raw = serde_json::to_string(&tool).unwrap();
    let form = store_block_form(&tool, &raw, &rows).unwrap();
    assert_eq!(
        form.skeleton
            .matches(&format!("\"{PAYLOAD_PLACEHOLDER}\""))
            .count(),
        2
    );
    assert_eq!(form.payloads.len(), 2);
    // Each payload is the member's serialized JSON value, so the two stripped
    // members keep their document order and their exact content.
    assert_eq!(
        form.payloads[0].matches('x').count(),
        CHUNK_TARGET_BYTES + 1
    );
    assert_eq!(
        form.payloads[1].matches('y').count(),
        CHUNK_TARGET_BYTES + 2
    );
    assert_eq!(
        block_value(&form.skeleton, &form.elements, &form.payloads).unwrap(),
        tool
    );
    // The raw splice stays valid JSON with the segments substituted.
    let spliced = block_raw_json(
        &form.skeleton,
        &pack_stored_chunks(&form.elements, CHUNK_TARGET_BYTES),
        &form.payloads,
    )
    .unwrap();
    assert!(serde_json::from_str::<Value>(&spliced).is_ok());
    // Segment pieces are bounded and carry no markers.
    for payload in &form.payloads {
        for piece in pack_payload_segments(payload, CHUNK_TARGET_BYTES) {
            assert!(piece.len() <= CHUNK_TARGET_BYTES);
            assert!(!piece.starts_with(super::layout::CHUNK_CONTINUATION));
        }
    }
}

#[test]
fn oversized_nested_tool_strings_keep_the_skeleton_small_and_round_trip() {
    let text = "é漢🙂".repeat(CHUNK_TARGET_BYTES / 8 + 17);
    let mut block = large_tool_block(&text);
    block["text"] = json!(text);
    block["tool"]["title"] = json!(text);
    block["tool"]["preview"] = json!({"title":text,"future":[{"body":text}]});
    block["futureMetadata"] = json!({"nested":[text],"small":true});
    let rows = blocks::project(std::slice::from_ref(&block)).unwrap();
    let raw = serde_json::to_string(&block).unwrap();
    let form = store_block_form(&block, &raw, &rows).unwrap();
    assert!(form.skeleton.len() < 4096);
    assert_eq!(form.payloads.len(), 6);
    for payload in &form.payloads {
        assert!(pack_payload_segments(payload, CHUNK_TARGET_BYTES)
            .iter()
            .all(|piece| piece.len() <= CHUNK_TARGET_BYTES));
    }
    assert_eq!(
        block_value(
            &form.skeleton,
            &pack_stored_chunks(&form.elements, CHUNK_TARGET_BYTES),
            &form.payloads
        )
        .unwrap(),
        block
    );
    let hidden =
        json!({"id":"hidden-large","role":"assistant","draft":true,"text":text,"future":[text]});
    let raw = serde_json::to_string(&hidden).unwrap();
    let form = store_block_form(&hidden, &raw, &[]).unwrap();
    assert!(form.skeleton.len() < 4096);
    assert_eq!(
        block_value(&form.skeleton, &[], &form.payloads).unwrap(),
        hidden
    );
}

#[test]
fn giant_root_id_stays_inline_while_nested_ids_use_payloads() {
    let text = "identifier".repeat(CHUNK_TARGET_BYTES / 8);
    let block = json!({"id":text,"role":"assistant","draft":true,
        "text":"hidden","future":{"id":text}});
    let raw = serde_json::to_string(&block).unwrap();
    let form = store_block_form(&block, &raw, &[]).unwrap();
    assert!(form
        .skeleton
        .contains(&format!("\"id\":{}", serde_json::to_string(&text).unwrap())));
    assert_eq!(form.payloads.len(), 1);
    assert_eq!(
        block_value(&form.skeleton, &[], &form.payloads).unwrap(),
        block
    );
}

#[test]
fn literal_markers_survive_nested_payloads_and_hidden_text() {
    let block = json!({
        "id": "reserved-large",
        "role": "assistant",
        "draft": true,
        "text": "%NATIVE_ROWS_TEXT%",
        "future": {
            "%NATIVE_ROWS%/~": [
                format!("{}%NATIVE_ROWS%", "😀".repeat(CHUNK_TARGET_BYTES / 4)),
                "%NATIVE_PAYLOAD%",
                format!("%NATIVE_PAYLOAD%{}", "x".repeat(CHUNK_TARGET_BYTES)),
            ]
        }
    });
    let raw = serde_json::to_string(&block).unwrap();
    let form = store_block_form(&block, &raw, &[]).unwrap();
    assert!(form.skeleton.len() < CHUNK_TARGET_BYTES);
    assert_eq!(
        block_value(&form.skeleton, &[], &form.payloads).unwrap(),
        block
    );
}

#[test]
fn literal_markers_save_to_a_legacy_chunked_session_and_survive_reopen() {
    use crate::tool::{ToolCall, ToolState};
    use crate::transcript::Transcript;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.db");
    let mut store = Store::open_monocode(&path).unwrap();
    store.prefer_chunked_storage(true).unwrap();
    let session = store.create("claude", "/workspace").unwrap();
    let original = store.persisted_session_id(session.id).unwrap();
    let mut transcript = Transcript::default();
    transcript.begin("assistant");
    transcript.append("Already saved");
    store.write_rows(session.id, 0, &transcript.rows).unwrap();
    assert_eq!(store.storage_mode_version(session.id).unwrap(), Some(1));
    let before = native_snapshot(&store.conn, &original);

    let prompt = transcript.begin("user");
    transcript.append("Block JSON contains the reserved marker %NATIVE_ROWS%, so its skeleton could not be spliced unambiguously");
    let tool = transcript.begin("tool");
    let mut call = ToolCall::new("call-example", "Read");
    call.output = "source line: const ROWS_PLACEHOLDER = \"%NATIVE_ROWS%\";".into();
    call.state = ToolState::Completed;
    transcript.rows[tool].text = serde_json::to_string(&call).unwrap();
    store.fail_next_save_commit_for_test().unwrap();
    assert!(store
        .write_rows(session.id, prompt, &transcript.rows[prompt..])
        .is_err());
    assert_eq!(store.storage_mode_version(session.id).unwrap(), Some(1));
    assert_eq!(native_snapshot(&store.conn, &original), before);
    store
        .write_rows(session.id, prompt, &transcript.rows[prompt..])
        .unwrap();

    let followup = transcript.begin("assistant");
    transcript.append("Later reply");
    store
        .write_rows(session.id, followup, &transcript.rows[followup..])
        .unwrap();
    let after = native_snapshot(&store.conn, &original);
    assert_eq!(store.storage_mode_version(session.id).unwrap(), Some(2));
    assert_eq!(
        (&after[0].1, &after[0].2, after[0].3),
        (&before[0].1, &before[0].2, before[0].3)
    );
    drop(store);

    let mut reopened = Store::open_monocode(&path).unwrap();
    let id = reopened.resolve_session_id(&original).unwrap();
    let rows = reopened.load(id).unwrap().rows;
    assert_eq!(rows[0].text, "Already saved");
    assert_eq!(rows[prompt].text, transcript.rows[prompt].text);
    assert_eq!(
        serde_json::from_str::<ToolCall>(&rows[tool].text)
            .unwrap()
            .output,
        call.output
    );
    assert_eq!(rows[followup].text, "Later reply");
    reopened.downgrade_session_to_blob(id).unwrap();
    assert_eq!(reopened.load(id).unwrap().rows, rows);
}

fn seed_legacy_large_tool(store: &mut Store, id: &str) -> Vec<Value> {
    let text = "x".repeat(2 * CHUNK_TARGET_BYTES + 123);
    let mut tool = large_tool_block(&text);
    tool["text"] = json!(text);
    tool["tool"]["title"] = json!(text);
    tool["tool"]["preview"] = json!({"title":text,"future":true});
    tool["tool"]["nativePresentation"] = json!({"title":"Synthetic active tool","kind":"read"});
    let values = vec![
        json!({"id":"p","role":"user","text":"Synthetic prompt","appRequestId":"fake-receipt"}),
        tool,
        json!({"id":"hidden","role":"assistant","draft":true,"text":text,"future":true}),
    ];
    seed_native_session(&store.conn, id, &[]);
    let updates = values
        .iter()
        .map(|block| {
            let raw = serde_json::to_string(block).unwrap();
            let rows = blocks::project(std::slice::from_ref(block)).unwrap();
            let form = super::layout::legacy_store_block_form(block, &raw, &rows).unwrap();
            BlockUpdate {
                skeleton: form.skeleton,
                rows: form.elements,
                payloads: form.payloads,
            }
        })
        .collect::<Vec<_>>();
    let tx = store.conn.transaction().unwrap();
    replace_blocks_tx(&tx, id, 0, &updates).unwrap();
    tx.commit().unwrap();
    values
}

#[test]
fn legacy_large_members_can_downgrade_without_a_normalization_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-export.db");
    let expected;
    let rows;
    {
        let mut store = Store::open_monocode(&path).unwrap();
        expected = seed_legacy_large_tool(&mut store, "legacy-export");
        let handle = store.resolve_session_id("legacy-export").unwrap();
        rows = store.load(handle).unwrap().rows;
        store.downgrade_session_to_blob(handle).unwrap();
        assert_eq!(store.stored_blocks(handle).unwrap(), expected);
        assert_eq!(
            native_row_counts(&store.conn, "legacy-export"),
            (0, 0, 0, 0)
        );
    }
    let store = Store::open_monocode(&path).unwrap();
    let handle = store.resolve_session_id("legacy-export").unwrap();
    assert_eq!(store.stored_blocks(handle).unwrap(), expected);
    assert_eq!(store.load(handle).unwrap().rows, rows);
}

#[test]
fn legacy_large_members_normalize_on_save_without_losing_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-save.db");
    let expected;
    let saved_rows;
    {
        let mut store = Store::open_monocode(&path).unwrap();
        let mut values = seed_legacy_large_tool(&mut store, "legacy-save");
        let handle = store.resolve_session_id("legacy-save").unwrap();
        let mut rows = store.load(handle).unwrap().rows;
        let index = rows.iter().position(|row| row.role == "tool").unwrap();
        let mut call: crate::tool::ToolCall = serde_json::from_str(&rows[index].text).unwrap();
        call.input.push_str(" appended 中文🙂");
        values[1]["text"] = json!(call.input);
        values[1]["tool"]["title"] = json!(call.input);
        values[1]["tool"]["nativeInput"] = json!(call.input);
        rows[index].text = serde_json::to_string(&call).unwrap();
        store
            .write_rows(handle, index, &rows[index..index + 1])
            .unwrap();
        assert!(native_snapshot(&store.conn, "legacy-save")[1].1.len() < 4096);
        assert_eq!(store.stored_blocks(handle).unwrap(), values);
        expected = values;
        saved_rows = store.load(handle).unwrap().rows;
    }
    let mut store = Store::open_monocode(&path).unwrap();
    let handle = store.resolve_session_id("legacy-save").unwrap();
    assert_eq!(store.stored_blocks(handle).unwrap(), expected);
    assert_eq!(store.load(handle).unwrap().rows, saved_rows);
    store.downgrade_session_to_blob(handle).unwrap();
    assert_eq!(store.stored_blocks(handle).unwrap(), expected);
}

#[test]
fn downgrade_refuses_noncanonical_forms_in_both_layouts() {
    for normalize in [false, true] {
        let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
        let expected = seed_legacy_large_tool(&mut store, "strict-export");
        let handle = store.resolve_session_id("strict-export").unwrap();
        store.load(handle).unwrap();
        if normalize {
            store.store_blocks(handle, &expected).unwrap();
        }
        // Still valid, reconstructible JSON with unchanged rows and metadata,
        // but neither canonical writer emits this leading whitespace.
        store.conn.execute(
            "UPDATE native_blocks SET skeleton_json=' '||skeleton_json WHERE session_id=?1 AND block_seq=1",
            ["strict-export"],
        ).unwrap();
        let before = native_snapshot(&store.conn, "strict-export");
        let blob = stored_blob(&store.conn, "strict-export");
        assert!(store
            .downgrade_session_to_blob(handle)
            .unwrap_err()
            .contains("would not re-store to its stored form"));
        assert_eq!(native_snapshot(&store.conn, "strict-export"), before);
        assert_eq!(stored_blob(&store.conn, "strict-export"), blob);
        assert_eq!(store.stored_blocks(handle).unwrap(), expected);
    }
}

#[test]
fn restart_equality_rows_and_raw_blocks() {
    let blocks = vec![
        json!({"id":"prompt","role":"user","text":"Write a report 🙂","sentAt":1,"futureMetadata":{"keep":[1,2]}}),
        json!({"id":"run","role":"tool","text":"Run checks","sentAt":2,
            "tool":{"callId":"call-1","nativeName":"Bash","nativeInput":"{\"command\":\"make check\"}",
                    "kind":"execute","title":"make check","status":"completed","futureToolField":[1]}}),
        json!({"id":"hidden","role":"assistant","text":"never displayed","draft":true,"futureHidden":"keep"}),
        json!({"id":"answer","role":"assistant","text":"Done 中文","sentAt":3}),
    ];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("restart.db");
    let original: String;
    let rows: Vec<crate::transcript::Row>;
    let stored: Vec<Value>;
    {
        let mut store = Store::open_monocode(&path).unwrap();
        let session = created_chunked(&store, "codex");
        original = store.persisted_session_id(session).unwrap();
        // The creation opt-in really landed the chunked marker, not a blob.
        let mode: i64 = store
            .conn
            .query_row(
                "SELECT mode_version FROM native_sessions WHERE session_id=?1",
                [&original],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mode, i64::from(INITIAL_NATIVE_CHUNKS_MODE_VERSION));
        store.shared_store_blocks(session, &blocks).unwrap();
        // A runtime-style streamed append lands through the sparse writer too.
        let seeded = blocks::project(&blocks).unwrap();
        store
            .write_rows(
                session,
                seeded.len(),
                &[crate::transcript::Row {
                    role: "assistant".into(),
                    text: "Tail after restart".into(),
                    first: true,
                    sent_at: Some(4),
                }],
            )
            .unwrap();
        rows = store.load(session).unwrap().rows;
        stored = iter_full_history(&store.conn, &original)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(rows.len(), seeded.len() + 1);
        // The rich fixture — hidden draft included — is what the reader returns.
        assert_eq!(&stored[..blocks.len()], &blocks[..]);
    }
    // Reopen: the same rows under the same numbers, the same raw blocks.
    let store = Store::open_monocode(&path).unwrap();
    let session = store.resolve_session_id(&original).unwrap();
    assert_eq!(store.load(session).unwrap().rows, rows);
    let reopened: Vec<Value> = iter_full_history(&store.conn, &original)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(reopened, stored);
    assert!(reopened.iter().any(|block| block["id"] == "hidden"));
    // The blob stayed the untouched creation fixture throughout.
    let fixture: String = store
        .conn
        .query_row(
            "SELECT blocks_json FROM sessions WHERE id=?1",
            [&original],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(fixture, "[]");
}

#[test]
fn pagination_does_not_shift_origin() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let blocks = vec![
        json!({"id":"p","role":"user","text":"Prompt","sentAt":1}),
        json!({"id":"long","role":"assistant","text":"z".repeat(10 * crate::transcript::ROW_BYTES),"sentAt":2}),
        json!({"id":"t","role":"tool","text":"Tool","sentAt":3,
            "tool":{"callId":"c","nativeName":"Read","nativeInput":"{}","kind":"read","title":"Tool","status":"completed"}}),
    ];
    let id = "page-origin";
    seed_native_session(&store.conn, id, &blocks);
    let handle = store.resolve_session_id(id).unwrap();
    let all = store.load(handle).unwrap().rows;
    assert!(all.len() >= 12);
    assert_eq!(store.page_rows(handle, 0, usize::MAX).unwrap(), all);
    // The same rows under the same numbers for every window shape.
    for (start, limit) in [(0_usize, 2_usize), (2, 2), (4, 2), (1, 3), (3, 1), (6, 4)] {
        assert_eq!(
            store.page_rows(handle, start, limit).unwrap(),
            &all[start.min(all.len())..(start + limit).min(all.len())],
            "window {start}+{limit}"
        );
    }
    // Row k is the same row in every window that ends on it.
    for (k, row) in all.iter().enumerate() {
        for start in 0..=k {
            let page = store.page_rows(handle, start, k - start + 1).unwrap();
            assert_eq!(page.last(), Some(row), "row {k} from {start}");
        }
    }
    // A tail append never renumbers what came before.
    store
        .write_rows(
            handle,
            all.len(),
            &[crate::transcript::Row {
                role: "user".into(),
                text: "Later".into(),
                first: true,
                sent_at: Some(9),
            }],
        )
        .unwrap();
    let grown = store.load(handle).unwrap().rows;
    assert_eq!(&grown[..all.len()], &all[..]);
    assert_eq!(store.page_rows(handle, 0, all.len()).unwrap(), all);
    assert_eq!(
        store.page_rows(handle, all.len(), 10).unwrap(),
        &grown[all.len()..]
    );
}

/// The probe's large-tool shape: one active tool block whose `nativeInput`
/// alone is ≥5 MiB, updated by appending to its input.
fn large_tool_block(input: &str) -> Value {
    json!({"id":"fixed-active","role":"tool","text":"Synthetic active tool","sentAt":2,
        "tool":{"callId":"fixed-active-call","nativeName":"Read","nativeInput":input,"kind":"read","title":"Synthetic active tool","status":"running","detail":"synthetic output","futureToolField":true},"futureMetadata":{"active":true}})
}

#[test]
fn large_tool_update_rewrites_bounded_bytes() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    // A tool already rendered by a previous update carries giant captions too.
    // First creation of those captions is covered separately below.
    let initial_input = "x".repeat(5 * 1024 * 1024);
    let mut active = large_tool_block(&initial_input);
    active["text"] = json!(initial_input);
    active["tool"]["title"] = json!(initial_input);
    active["tool"]["preview"] = json!({"title":initial_input,"future":true});
    active["tool"]["nativePresentation"] = json!({"title":"Synthetic active tool","kind":"read"});
    let blocks = vec![
        json!({"id":"prompt","role":"user","text":"Run the reader","sentAt":1}),
        active,
        json!({"id":"tail","role":"assistant","text":"Tail reply","sentAt":2}),
    ];
    let id = "large-tool";
    seed_native_session(&store.conn, id, &blocks);
    let handle = store.resolve_session_id(id).unwrap();
    let before = native_snapshot(&store.conn, id);
    // The block is genuinely giant: its single row element holds the whole
    // serialized ToolCall, and its `nativeInput` member is stripped out of the
    // skeleton into bounded payload segments.
    let row_bytes: u64 = before[1].2.iter().map(|chunk| chunk.len() as u64).sum();
    assert!(row_bytes >= 5 * 1024 * 1024);
    assert!(before[1].1.contains(PAYLOAD_PLACEHOLDER));
    assert_eq!(before[1].5.len(), 4);
    assert!(before[1].1.len() < 4096);
    assert!(before[1]
        .5
        .iter()
        .flatten()
        .all(|segment| segment.len() <= CHUNK_TARGET_BYTES));
    // Repeated small updates, streamed exactly as the runtime updates a tool:
    // parse the row's ToolCall, append to its input, re-serialize.
    let mut rows = blocks::project(&blocks).unwrap();
    let tool_row = rows
        .iter()
        .rposition(|row| row.first && row.role == "tool")
        .unwrap();
    let mut input = "x".repeat(5 * 1024 * 1024);
    for round in 0..4 {
        input.push_str(&"y".repeat(1024));
        let mut call: crate::tool::ToolCall = serde_json::from_str(&rows[tool_row].text).unwrap();
        call.input.push_str(&"y".repeat(1024));
        rows[tool_row].text = serde_json::to_string(&call).unwrap();
        store
            .write_row_ranges(handle, &[(tool_row, &rows[tool_row..tool_row + 1])])
            .unwrap();
        let sample = store.take_save_sample().expect("save sample");
        assert_eq!(sample.blocks_written, 1, "round {round}");
        // Only the tail pieces of the byte-split row and the tail segment of
        // the stripped member move: O(chunk target + update), never O(payload).
        assert!(sample.chunks_written <= 4, "round {round}");
        assert!(
            sample.chunk_bytes_written <= 4 * (CHUNK_TARGET_BYTES + 4096) as u64,
            "round {round}: {} bytes written",
            sample.chunk_bytes_written
        );
        assert!(sample.chunk_bytes_written >= 1024, "round {round}");
        assert!(sample.skeleton_bytes_written < 4096, "round {round}");
        let json_bytes = sample.skeleton_bytes_written + sample.chunk_bytes_written;
        assert!(json_bytes <= 4 * (CHUNK_TARGET_BYTES + 4096) as u64 + 4096);
        assert!(
            json_bytes * 16 < row_bytes,
            "round {round}: all transcript JSON writes must stay bounded"
        );
    }
    // Everything reads back byte-exact, through both reader entries.
    assert_eq!(native_rows(&store.conn, id), rows);
    let reopened: Vec<Value> = iter_full_history(&store.conn, id)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        reopened[1]["tool"]["nativeInput"].as_str(),
        Some(input.as_str())
    );
    assert_eq!(reopened[0], blocks[0]);
    // Accepting the very same row again writes nothing at all.
    let after_rounds = native_snapshot(&store.conn, id);
    store
        .write_row_ranges(handle, &[(tool_row, &rows[tool_row..tool_row + 1])])
        .unwrap();
    let sample = store.take_save_sample().expect("save sample");
    assert_eq!(sample.blocks_written, 0);
    assert_eq!(sample.chunk_bytes_written, 0);
    assert_eq!(sample.skeleton_bytes_written, 0);
    assert_eq!(native_snapshot(&store.conn, id), after_rounds);
}

#[test]
fn first_giant_captions_count_their_full_initial_write() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let input = "x".repeat(8 * CHUNK_TARGET_BYTES);
    let block = large_tool_block(&input);
    seed_native_session(&store.conn, "new-captions", std::slice::from_ref(&block));
    let id = store.resolve_session_id("new-captions").unwrap();
    let mut rows = store.load(id).unwrap().rows;
    for round in 0..2 {
        let mut call: crate::tool::ToolCall = serde_json::from_str(&rows[0].text).unwrap();
        call.input.push_str(&"y".repeat(1024));
        rows[0].text = serde_json::to_string(&call).unwrap();
        store.write_rows(id, 0, &rows).unwrap();
        let sample = store.take_save_sample().unwrap();
        let written = sample.chunk_bytes_written + sample.skeleton_bytes_written;
        if round == 0 {
            // New full-size captions are real new content; never hide this cost.
            assert!(written >= 2 * input.len() as u64);
        } else {
            assert!(written <= 4 * (CHUNK_TARGET_BYTES + 4096) as u64 + 4096);
        }
        assert!(sample.skeleton_bytes_written < 4096);
    }
}

#[test]
fn acceptance_lowers_is_draft_only_when_the_last_draft_goes() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let session = created_chunked(&store, "codex");
    // Two queued drafts and no sent message yet: the session lists as a
    // draft, exactly as the blob store's whole-list recompute would flag it.
    store
        .shared_store_blocks(
            session,
            &[
                json!({"id":"queued-1","role":"user","text":"First","draft":true}),
                json!({"id":"queued-2","role":"user","text":"Second","draft":true}),
            ],
        )
        .unwrap();
    assert!(store.draft_sessions().unwrap().contains(&session));
    // Accepting one draft keeps the other queued: still a draft.
    store
        .write_rows_with_message(
            session,
            0,
            &[crate::transcript::Row {
                role: "user".into(),
                text: "First".into(),
                first: true,
                sent_at: Some(1),
            }],
            Some("queued-1"),
        )
        .unwrap();
    assert!(store.draft_sessions().unwrap().contains(&session));
    // Accepting the last one clears the flag, matching the blob save.
    store
        .write_rows_with_message(
            session,
            1,
            &[crate::transcript::Row {
                role: "user".into(),
                text: "Second".into(),
                first: true,
                sent_at: Some(2),
            }],
            Some("queued-2"),
        )
        .unwrap();
    assert!(!store.draft_sessions().unwrap().contains(&session));
}

#[test]
fn side_thread_cleanup_and_snapshot_full_history() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    let session = created_chunked(&store, "codex");
    let original = store.persisted_session_id(session).unwrap();
    let blocks = vec![
        json!({"id":"p","role":"user","text":"Write a report","sentAt":1,
            "btwThreads":[{"id":"side-1","providerThreadId":"side-thread-1","harness":"codex",
                "sourceEndBlockId":"a2","createdAt":1,"status":"ready",
                "messages":[{"role":"user","text":"side question","createdAt":2},
                            {"role":"assistant","text":"side answer","createdAt":3}]}]}),
        json!({"id":"a1","role":"assistant","text":"First half","sentAt":2}),
        json!({"id":"a2","role":"assistant","text":"Second half","sentAt":3}),
        json!({"id":"later","role":"user","text":"Follow up","sentAt":4}),
    ];
    store.shared_store_blocks(session, &blocks).unwrap();
    // The side-thread readers walk the full chunked history.
    let threads = store.desktop_side_threads(session).unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].id, "side-1");
    assert_eq!(threads[0].provider, "codex");
    assert_eq!(
        threads[0].snapshot,
        "User: Write a report\n\nAssistant: First half\n\nAssistant: Second half"
    );
    let texts: Vec<String> = threads[0]
        .rows
        .iter()
        .map(|row| format!("{}:{}", row.role, row.text))
        .collect();
    assert_eq!(texts, ["user:side question", "assistant:side answer"]);
    assert_eq!(
        store.codex_side_thread_ids(session, "codex").unwrap(),
        vec!["side-thread-1".to_string()]
    );
    // Cleanup: deleting the session removes every native row with it.
    store.delete(session).unwrap();
    for table in [
        "native_sessions",
        "native_blocks",
        "native_chunks",
        "native_payloads",
    ] {
        assert_eq!(
            stored_block_count(&store.conn, &original, table),
            0,
            "{table}"
        );
    }
    let gone: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM sessions WHERE id=?1",
            [&original],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(gone, 0);
    assert!(store.desktop_side_threads(session).is_err());
}

#[test]
fn fork_edit_delete_restore_on_native() {
    let blocks = vec![
        json!({"id":"initial","role":"user","text":"Earlier","providerTurnId":"old","startedAt":100,
            "turnModel":{"harness":"codex","id":"codex:m","name":"M"}}),
        json!({"id":"answer","role":"assistant","text":"Kept","future":"keep"}),
        json!({"id":"original","role":"user","text":"Original turn","providerTurnId":"turn"}),
        json!({"id":"replaced","role":"assistant","text":"Old answer"}),
        json!({"id":"third","role":"user","text":"Third turn","providerTurnId":"turn3"}),
        json!({"id":"third-answer","role":"assistant","text":"Third answer"}),
    ];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fork.db");
    let original: String;
    let mut a = Store::open_monocode(&path).unwrap();
    let session = {
        a.prefer_chunked_storage(true).unwrap();
        let session = a.create("codex", "/project").unwrap().id;
        original = a.persisted_session_id(session).unwrap();
        session
    };
    a.provider_id(session, "thread").unwrap();
    a.shared_store_blocks(session, &blocks).unwrap();
    // EDIT: rewinding the last turn truncates the chunked history in place,
    // keeping the same absolute row numbers for everything that stays.
    let rows = a.load(session).unwrap().rows;
    let edit = a.prepare_edit_last_turn(session, &rows, None).unwrap();
    assert_eq!(edit.start, 4);
    a.finish_edit_last_turn(session, &edit, "thread").unwrap();
    assert_eq!(a.shared_blocks(session).unwrap(), blocks[..4]);
    assert_eq!(a.load(session).unwrap().rows, rows[..4]);
    assert_eq!(stored_block_count(&a.conn, &original, "native_blocks"), 4);
    assert_eq!(stored_block_count(&a.conn, &original, "native_payloads"), 0);
    // STALE: another window's save moves the revision; the prepared edit
    // refuses and the stored history is preserved untouched.
    let rows = a.load(session).unwrap().rows;
    let stale = a.prepare_edit_last_turn(session, &rows, None).unwrap();
    assert_eq!(stale.start, 2);
    {
        let mut b = Store::open_monocode(&path).unwrap();
        let other = b.resolve_session_id(&original).unwrap();
        let current = b.load(other).unwrap().rows;
        b.write_rows(
            other,
            current.len(),
            &[crate::transcript::Row {
                role: "assistant".into(),
                text: "another window".into(),
                first: true,
                sent_at: Some(9),
            }],
        )
        .unwrap();
    }
    let grown = a.shared_blocks(session).unwrap();
    assert_eq!(grown.len(), 5);
    assert!(a.finish_edit_last_turn(session, &stale, "fork").is_err());
    assert_eq!(a.shared_blocks(session).unwrap(), grown);
    // The refused finish leaves the durable edit-recovery guard; clearing it
    // is the app's recovery step, not a store change.
    a.save_setting(&format!("edit-recovery:{session}"), "")
        .unwrap();
    // FORK: a fresh edit lands, truncates to the same prefix and rebinds the
    // provider session id.
    let rows = a.load(session).unwrap().rows;
    let fork = a.prepare_edit_last_turn(session, &rows, None).unwrap();
    a.finish_edit_last_turn(session, &fork, "fork-2").unwrap();
    assert_eq!(a.shared_blocks(session).unwrap(), blocks[..2]);
    assert_eq!(a.load(session).unwrap().rows, rows[..2]);
    assert_eq!(
        a.sessions()
            .unwrap()
            .into_iter()
            .find(|s| s.id == session)
            .unwrap()
            .provider_id
            .as_deref(),
        Some("fork-2")
    );
    // RESTORE: a cloud restore writes the whole block list back through the
    // same store entry and reads back identical.
    a.shared_store_blocks(session, &blocks).unwrap();
    assert_eq!(a.shared_blocks(session).unwrap(), blocks);
    assert_eq!(
        a.load(session).unwrap().rows,
        blocks::project(&blocks).unwrap()
    );
    // DELETE: cleanup removes every native row.
    a.delete(session).unwrap();
    for table in [
        "native_sessions",
        "native_blocks",
        "native_chunks",
        "native_payloads",
    ] {
        assert_eq!(stored_block_count(&a.conn, &original, table), 0, "{table}");
    }
}

// ---- Task 7: per-session conversion, downgrade export, recovery ----

/// A mixed fixture for conversion: unknown metadata, a receipt, a hidden
/// draft, and a tool whose payload is larger than the chunk target, so a
/// conversion exercises skeleton storage, chunk packing and payload stripping
/// together.
fn conversion_fixture_blocks() -> Vec<Value> {
    let giant_input = "x".repeat(5 * CHUNK_TARGET_BYTES / 4);
    vec![
        json!({"id":"conv-prompt","role":"user","text":"Write a report on the synthetic fixture","sentAt":FIXED_TIME,
            "appRequestId":"conv-delivered","futureMetadata":{"keep":[1,2,3]}}),
        json!({"id":"conv-answer","role":"assistant","text":"First draft of the answer","futureField":true}),
        json!({"id":"conv-hidden","role":"assistant","text":"never displayed","draft":true,"futureHidden":"keep"}),
        json!({"id":"conv-tool","role":"tool","text":"Synthetic tool","sentAt":FIXED_TIME,
            "tool":{"callId":"conv-call","nativeName":"Bash","nativeInput":giant_input,"kind":"exec","title":"Synthetic tool","status":"completed"}}),
    ]
}

fn stored_blob(conn: &Connection, id: &str) -> String {
    conn.query_row(
        "SELECT blocks_json FROM sessions WHERE id=?1",
        [id],
        |row| row.get(0),
    )
    .unwrap()
}

/// The session's row counts across all four native tables.
fn native_row_counts(conn: &Connection, id: &str) -> (i64, i64, i64, i64) {
    conn.query_row(
        "SELECT (SELECT count(*) FROM native_sessions WHERE session_id=?1),
                (SELECT count(*) FROM native_blocks WHERE session_id=?1),
                (SELECT count(*) FROM native_chunks WHERE session_id=?1),
                (SELECT count(*) FROM native_payloads WHERE session_id=?1)",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .unwrap()
}

/// Everything an interrupted or refused conversion must leave behind: blob
/// mode, no marker, no native rows in any table, and the blob byte-identical.
fn conversion_rolled_back(conn: &Connection, id: &str, blob_before: &str) {
    assert_eq!(
        session_storage_mode(conn, id).unwrap(),
        StorageMode::Blob,
        "the session must still be a blob"
    );
    assert_eq!(native_row_counts(conn, id), (0, 0, 0, 0));
    assert_eq!(stored_blob(conn, id), blob_before);
}

/// One appended row, distinct per call site by its text.
fn appended_row(text: &str) -> crate::transcript::Row {
    crate::transcript::Row {
        role: "assistant".into(),
        text: text.into(),
        first: true,
        sent_at: Some(FIXED_TIME),
    }
}

#[test]
fn interrupted_conversion_keeps_blob_valid() {
    let blocks = conversion_fixture_blocks();
    // The marker stage: the marker is in, nothing else written yet.
    {
        let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
        insert_session(&store.conn, "conv", &blocks);
        let blob = stored_blob(&store.conn, "conv");
        let id = store.resolve_session_id("conv").unwrap();
        fail_next_conversion_at_for_test(ConversionStage::Marker);
        let error = store.convert_session_to_chunked(id).unwrap_err();
        assert!(error.contains("interruption"), "{error}");
        assert!(error.contains("still valid in its blob format"), "{error}");
        conversion_rolled_back(&store.conn, "conv", &blob);
        // The blob session still loads and saves exactly as before.
        let rows = store.load(id).unwrap().rows;
        store.write_rows(id, 0, &rows).unwrap();
        assert_eq!(store.load(id).unwrap().rows, rows);
    }
    // The chunk-build stage: every block, chunk and payload segment is
    // written, the commit is not reached.
    {
        let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
        insert_session(&store.conn, "conv", &blocks);
        let blob = stored_blob(&store.conn, "conv");
        let id = store.resolve_session_id("conv").unwrap();
        fail_next_conversion_at_for_test(ConversionStage::ChunkBuild);
        let error = store.convert_session_to_chunked(id).unwrap_err();
        assert!(error.contains("interruption"), "{error}");
        conversion_rolled_back(&store.conn, "conv", &blob);
    }
    // The commit itself, through the same hook a failing disk would take.
    {
        let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
        insert_session(&store.conn, "conv", &blocks);
        let blob = stored_blob(&store.conn, "conv");
        let id = store.resolve_session_id("conv").unwrap();
        store.fail_next_save_commit_for_test().unwrap();
        let error = store.convert_session_to_chunked(id).unwrap_err();
        assert!(error.contains("can be retried"), "{error}");
        conversion_rolled_back(&store.conn, "conv", &blob);
        let rows = store.load(id).unwrap().rows;
        store.write_rows(id, 0, &rows).unwrap();
        assert_eq!(store.load(id).unwrap().rows, rows);
    }
}

#[test]
fn conversion_is_atomic_and_retryable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("convert.db");
    let blocks = conversion_fixture_blocks();
    let mut store = Store::open_monocode(&path).unwrap();
    insert_session(&store.conn, "conv", &blocks);
    let blob = stored_blob(&store.conn, "conv");
    let id = store.resolve_session_id("conv").unwrap();
    let rows_before = store.load(id).unwrap().rows;
    // First attempt dies after every chunk landed; nothing survives it.
    fail_next_conversion_at_for_test(ConversionStage::ChunkBuild);
    store.convert_session_to_chunked(id).unwrap_err();
    conversion_rolled_back(&store.conn, "conv", &blob);
    // Retry converts the same session cleanly, through the writer's own
    // full-replace path (canonical packing, never a hand-rolled layout).
    let report = store.convert_session_to_chunked(id).unwrap();
    assert_eq!(report.blocks, blocks.len() as u64);
    assert!(report.chunks >= 2, "{}", report.chunks);
    // The giant tool input is split at the target and its payload stripped
    // into segments: the stored form is the canonical one later saves anchor.
    let tool_chunks: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM native_chunks WHERE session_id='conv' AND block_seq=3",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(tool_chunks >= 2, "the oversized row is split");
    let payloads = native_row_counts(&store.conn, "conv").3;
    assert!(payloads >= 2, "the stripped member is segmented");
    assert_eq!(
        session_storage_mode(&store.conn, "conv").unwrap(),
        StorageMode::NativeChunks {
            mode_version: INITIAL_NATIVE_CHUNKS_MODE_VERSION
        }
    );
    // The blob stays in place, byte-identical — the marker switched the
    // readers — and the native reader serves exactly the same rows.
    assert_eq!(stored_blob(&store.conn, "conv"), blob);
    assert_eq!(store.load(id).unwrap().rows, rows_before);
    // The conversion cost is reported, not hidden: the chunk and payload
    // bytes alone are at least the blob's own bytes.
    assert!(report.bytes_written >= blob.len() as u64);
    // The consistency backup sits beside the database with the documented
    // deterministic name and holds exactly the pre-conversion blob.
    let backup = std::path::PathBuf::from(format!(
        "{}.pre-native-{}.backup",
        path.display(),
        handle("conv")
    ));
    let raw: String = Connection::open(&backup)
        .unwrap()
        .query_row(
            "SELECT blocks_json FROM sessions WHERE id='conv'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(raw, blob);
    // A second conversion of the same session refuses distinctly.
    let error = store.convert_session_to_chunked(id).unwrap_err();
    assert!(
        error.contains("already stored in the chunked backend"),
        "{error}"
    );
    // The revision guard was seeded by the conversion itself: the same handle
    // can keep saving sparsely without a reload.
    let mut rows = rows_before.clone();
    let appended = appended_row("appended after conversion");
    rows.push(appended.clone());
    store
        .write_rows(id, rows.len() - 1, std::slice::from_ref(&appended))
        .unwrap();
    assert_eq!(store.load(id).unwrap().rows, rows);
    // A fresh handle reads the converted session identically.
    drop(store);
    let store = Store::open_monocode(&path).unwrap();
    let id = store.resolve_session_id("conv").unwrap();
    assert_eq!(store.load(id).unwrap().rows, rows);
    assert_eq!(stored_blob(&store.conn, "conv"), blob);
}

#[test]
fn downgrade_export_restores_latest_and_verifies() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    insert_session(&store.conn, "conv", &conversion_fixture_blocks());
    let id = store.resolve_session_id("conv").unwrap();
    store.convert_session_to_chunked(id).unwrap();
    // Life as a chunked session: the latest messages land through the sparse
    // writer, after the blob was left behind.
    let mut rows = store.load(id).unwrap().rows;
    let latest = appended_row("the latest answer");
    rows.push(latest.clone());
    store
        .write_rows(id, rows.len() - 1, std::slice::from_ref(&latest))
        .unwrap();
    let blob_before = stored_blob(&store.conn, "conv");
    // A tampered stored chunk makes the restart-equality verification refuse:
    // a valid but different row would not match the rebuilt blob's rows.
    let original_chunk: String = store
        .conn
        .query_row(
            "SELECT rows_json FROM native_chunks WHERE session_id='conv' AND block_seq=0 AND chunk_ord=0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let tampered = format!(
        "{{\"role\":\"user\",\"text\":\"tampered row\",\"first\":true,\"sentAt\":{FIXED_TIME}}}"
    );
    store
        .conn
        .execute(
            "UPDATE native_chunks SET rows_json=?2 WHERE session_id=?1 AND block_seq=0 AND chunk_ord=0",
            params!["conv", tampered],
        )
        .unwrap();
    let error = store.downgrade_session_to_blob(id).unwrap_err();
    assert!(
        error.contains("Restart-equality verification failed"),
        "{error}"
    );
    assert_eq!(
        session_storage_mode(&store.conn, "conv").unwrap(),
        StorageMode::NativeChunks {
            mode_version: INITIAL_NATIVE_CHUNKS_MODE_VERSION
        }
    );
    assert_eq!(native_row_counts(&store.conn, "conv").0, 1);
    assert_eq!(stored_blob(&store.conn, "conv"), blob_before);
    // Repair the chunk: the downgrade then restores the latest content as a
    // blob and leaves no native residue behind the Blob marker.
    store
        .conn
        .execute(
            "UPDATE native_chunks SET rows_json=?2 WHERE session_id=?1 AND block_seq=0 AND chunk_ord=0",
            params!["conv", original_chunk],
        )
        .unwrap();
    store.downgrade_session_to_blob(id).unwrap();
    assert_eq!(
        session_storage_mode(&store.conn, "conv").unwrap(),
        StorageMode::Blob
    );
    assert_eq!(native_row_counts(&store.conn, "conv"), (0, 0, 0, 0));
    let downgraded: Vec<Value> = serde_json::from_str(&stored_blob(&store.conn, "conv")).unwrap();
    assert_eq!(blocks::project(&downgraded).unwrap(), rows);
    // The blob path's session-list flags were recomputed as a blob save would.
    let (has_user, is_draft): (i64, i64) = store
        .conn
        .query_row(
            "SELECT has_user_message,is_draft FROM sessions WHERE id='conv'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((has_user, is_draft), (1, 0));
    // A same-handle load and save work right away: the conversion-time guards
    // were invalidated by the operation that flipped the marker back.
    assert_eq!(store.load(id).unwrap().rows, rows);
    let edited = appended_row("after the downgrade");
    rows.push(edited.clone());
    store
        .write_rows(id, rows.len() - 1, std::slice::from_ref(&edited))
        .unwrap();
    assert_eq!(store.load(id).unwrap().rows, rows);
    assert_ne!(stored_blob(&store.conn, "conv"), blob_before);
    // Restart equality held, so converting back reproduces the same content —
    // the round trip loses nothing.
    store.convert_session_to_chunked(id).unwrap();
    assert_eq!(store.load(id).unwrap().rows, rows);
}

#[test]
fn old_blob_writer_cannot_silently_dual_write() {
    let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
    insert_session(&store.conn, "conv", &conversion_fixture_blocks());
    let blob = stored_blob(&store.conn, "conv");
    let id = store.resolve_session_id("conv").unwrap();
    store.convert_session_to_chunked(id).unwrap();
    // Every blob-shaped write entry now routes through the chunked backend —
    // the sparse save, the acceptance save, the multi-range save and the
    // full-replace store — and none of them writes `blocks_json` in parallel.
    let mut rows = store.load(id).unwrap().rows;
    rows[1].text = "edited answer".into();
    store.write_rows(id, 1, &rows[1..2]).unwrap();
    let accepted = crate::transcript::Row {
        role: "user".into(),
        text: "second prompt".into(),
        first: true,
        sent_at: Some(FIXED_TIME),
    };
    store
        .write_rows_with_message(
            id,
            rows.len(),
            std::slice::from_ref(&accepted),
            Some("msg-1"),
        )
        .unwrap();
    store.write_row_ranges(id, &[(0, &rows[..1])]).unwrap();
    let mut current = store.stored_blocks(id).unwrap();
    current[0]["text"] = json!("full replace text");
    store.store_blocks(id, &current).unwrap();
    assert_eq!(stored_blob(&store.conn, "conv"), blob, "no dual write");
    // Every write landed in the native backend: the reload reflects them all.
    let reloaded = store.load(id).unwrap().rows;
    assert_eq!(reloaded[0].text, "full replace text");
    assert_eq!(reloaded[1].text, "edited answer");
    assert!(reloaded.iter().any(|row| row.text == "second prompt"));
    // The blob writer's own gate refuses with the distinct kind when the
    // marker contradicts it — the mid-flight flip an old client cannot see.
    let error = store_blob_blocks(&store.conn, "conv", "[]").unwrap_err();
    assert!(error.contains(UNSUPPORTED_STORAGE_MODE), "{error}");
    assert_eq!(stored_blob(&store.conn, "conv"), blob);
    // A marker-less session still writes through the same entry.
    insert_session(
        &store.conn,
        "plain",
        &[json!({"id":"p","role":"user","text":"Plain","sentAt":1})],
    );
    store_blob_blocks(
        &store.conn,
        "plain",
        "[{\"id\":\"p\",\"role\":\"user\",\"text\":\"Plain twice\",\"sentAt\":1}]",
    )
    .unwrap();
    assert!(stored_blob(&store.conn, "plain").contains("Plain twice"));
}
