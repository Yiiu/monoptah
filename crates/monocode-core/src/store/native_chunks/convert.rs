//! Per-session conversion between the desktop blob and the chunked backend.
//!
//! [`convert_session_to_native`] switches one ordinary blob session over: a
//! consistency backup of the whole database taken from its committed state,
//! then one `BEGIN IMMEDIATE` transaction that renders every block into the
//! writer's own canonical stored form, inserts the mode marker and commits.
//! The blob column is left in place, untouched — the marker switches the
//! readers — so a rollback at any point leaves the old format exactly as it
//! was, and a retry starts clean.
//!
//! [`export_downgrade_to_blob`] is the verifiable way back: it rebuilds
//! `blocks_json` from the latest native content, proves restart equality (the
//! rebuilt blob projects to exactly the rows the backend holds, and every
//! block re-stores to its exact stored form) and deletes the native copy in
//! the same transaction that restores the blob, so a stale native copy can
//! never sit behind a Blob marker and resurrect if the marker flips again.
use super::layout::{
    legacy_store_block_form, pack_payload_segments, pack_stored_chunks, StoredForm,
    CHUNK_TARGET_BYTES,
};
use super::writer::{replace_blocks_tx, BlockUpdate};
use super::{
    iter_full_history, marker_mode_version, page_rows, session_storage_mode, store_block_form,
    StorageMode,
};
use crate::store::shared::{blocks, handle, HistoryRevision};
use rusqlite::{Connection, OptionalExtension};
use std::path::PathBuf;

/// What one conversion wrote: the block rows, the chunk and payload-segment
/// writes, and the bytes those carried. Skeleton bytes, the marker row and
/// the (once per attempt) whole-database backup are not counted — the numbers
/// report what the conversion itself stored. Public like `DirtyBlock`: the
/// runtime reports it after converting a session it owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversionReport {
    pub blocks: u64,
    pub chunks: u64,
    pub bytes_written: u64,
    /// The revision committed by this conversion, before any subsequent writer.
    pub session_revision: i64,
    pub(crate) mode_version: u32,
}

/// Every way a conversion refuses. Distinct refusals — a session that is not
/// an ordinary blob, a block the layout layer cannot store, a backup that
/// could not be taken or verified — are surfaced as themselves, never folded
/// into a silent skip or a partial conversion.
#[derive(Debug)]
pub(crate) enum ConversionError {
    /// No such session in the desktop schema.
    UnknownSession,
    /// The session already carries a chunked-storage marker.
    AlreadyNative,
    /// The session is an indexed history with its own ownership rules.
    IndexedHistory,
    /// A block refuses chunked storage (for example, it has its own `rows`
    /// member). The whole session is refused, never one block.
    Refused(String),
    /// The writer refused the converted block list (missing or duplicate ids).
    Writer(String),
    /// The consistency backup could not be taken or verified.
    Backup(String),
    Sqlite(rusqlite::Error),
    /// Test injection only: an interruption at a named stage.
    #[cfg(test)]
    Interrupted(ConversionStage),
}
impl From<rusqlite::Error> for ConversionError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}
impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSession => write!(
                f,
                "This conversation does not exist in the desktop schema; nothing was converted"
            ),
            Self::AlreadyNative => write!(
                f,
                "This conversation is already stored in the chunked backend; nothing to convert"
            ),
            Self::IndexedHistory => write!(
                f,
                "This conversation is an indexed history with its own ownership rules; it cannot \
                 be converted to chunked storage"
            ),
            Self::Refused(reason) => write!(
                f,
                "The conversion to chunked storage was refused: {reason}. The conversation is \
                 unchanged and still valid in its blob format"
            ),
            Self::Writer(reason) => write!(
                f,
                "The conversion to chunked storage was refused by the writer: {reason}. The \
                 conversation is unchanged and still valid in its blob format"
            ),
            Self::Backup(reason) => write!(
                f,
                "A consistent backup could not be taken before converting this conversation: \
                 {reason}. Nothing was converted"
            ),
            Self::Sqlite(error) => write!(
                f,
                "Converting this conversation failed: {error}. It is unchanged and still valid \
                 in its blob format; the conversion can be retried"
            ),
            #[cfg(test)]
            Self::Interrupted(stage) => {
                write!(
                    f,
                    "injected interruption at the {stage:?} stage of the conversion; it rolled \
                     back, and the conversation is unchanged and still valid in its blob format"
                )
            }
        }
    }
}

/// The stages a conversion can be interrupted at, for the crash-safety tests.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConversionStage {
    /// Right after the mode marker insert, before any chunk is written.
    Marker,
    /// After every chunk landed, before the commit.
    ChunkBuild,
}

#[cfg(test)]
std::thread_local! {
    static FAIL_NEXT_STAGE: std::cell::RefCell<Option<ConversionStage>> =
        const { std::cell::RefCell::new(None) };
    static AFTER_BACKUP_COPY: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Arm a one-shot failure at a conversion stage (test only).
#[cfg(test)]
pub(crate) fn fail_next_conversion_at_for_test(stage: ConversionStage) {
    FAIL_NEXT_STAGE.set(Some(stage));
}

/// Whether the armed failure targets this stage; consuming it only on a match,
/// so an injection armed for a later stage survives an earlier checkpoint.
#[cfg(test)]
fn fail_if_injected(stage: ConversionStage) -> bool {
    FAIL_NEXT_STAGE.with(|armed| {
        let matches = matches!(*armed.borrow(), Some(armed) if armed == stage);
        if matches {
            *armed.borrow_mut() = None;
        }
        matches
    })
}

/// Pages copied per backup step: large enough that a database of this app
/// copies in one step, so the backup is one consistent snapshot. The pause is
/// only taken on BUSY/LOCKED, when another connection holds the source.
const BACKUP_PAGES_PER_STEP: i32 = 100_000;
const BACKUP_BUSY_PAUSE: std::time::Duration = std::time::Duration::from_millis(5);

/// Convert one ordinary blob session to the chunked backend. One
/// `BEGIN IMMEDIATE` transaction covers the marker insert and every stored
/// block, so an interruption — injected, BUSY, FULL, a failed commit — rolls
/// the whole thing back and leaves the blob session exactly as it was; the
/// conversion can then be retried. The consistency backup runs strictly
/// before the transaction, from the committed state: SQLite's backup API is
/// unspecified against an open write transaction on the source connection,
/// and the atomicity requirement is about the conversion itself — the backup
/// is the safety net beside it.
pub(crate) fn convert_session_to_native(
    conn: &mut Connection,
    session_id: &str,
    expected: Option<HistoryRevision>,
) -> Result<ConversionReport, ConversionError> {
    // Refuse before touching anything: the session must be an ordinary blob.
    let source_version: i64 = conn.query_row("PRAGMA data_version", [], |row| row.get(0))?;
    let committed: Option<String> = conn
        .query_row(
            "SELECT blocks_json FROM sessions WHERE id=?1",
            [session_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(committed) = committed else {
        return Err(ConversionError::UnknownSession);
    };
    if marker_mode_version(conn, session_id)?.is_some() {
        return Err(ConversionError::AlreadyNative);
    }
    if indexed_history(conn, session_id)? {
        return Err(ConversionError::IndexedHistory);
    }
    let backup = take_consistency_backup(conn, session_id, &committed)?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    match convert_tx(
        conn,
        session_id,
        &committed,
        source_version,
        expected,
        backup,
    ) {
        Ok(report) => {
            // The chunk-build stage: every block, chunk and payload segment is
            // written but uncommitted.
            #[cfg(test)]
            {
                if fail_if_injected(ConversionStage::ChunkBuild) {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(ConversionError::Interrupted(ConversionStage::ChunkBuild));
                }
            }
            match conn.execute_batch("COMMIT") {
                Ok(()) => Ok(report),
                Err(error) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    Err(ConversionError::Sqlite(error))
                }
            }
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// The conversion inside the open transaction. `backed_up` is the committed
/// blob the pre-transaction backup verified: if the session moved while that
/// backup was taken, the conversion refuses rather than shadow a state its
/// safety net does not hold.
fn convert_tx(
    conn: &Connection,
    session_id: &str,
    backed_up: &str,
    source_version: i64,
    expected: Option<HistoryRevision>,
    backup: Option<ConsistencyBackup>,
) -> Result<ConversionReport, ConversionError> {
    // BEGIN IMMEDIATE holds the write lock, so these re-reads see exactly the
    // state the conversion builds on.
    if marker_mode_version(conn, session_id)?.is_some() {
        return Err(ConversionError::AlreadyNative);
    }
    if indexed_history(conn, session_id)? {
        return Err(ConversionError::IndexedHistory);
    }
    let raw: String = conn.query_row(
        "SELECT blocks_json FROM sessions WHERE id=?1",
        [session_id],
        |row| row.get(0),
    )?;
    let current_version: i64 = conn.query_row("PRAGMA data_version", [], |row| row.get(0))?;
    if raw != backed_up || current_version != source_version {
        return Err(ConversionError::Backup(
            "the database changed while its backup was taken; retry the conversion".into(),
        ));
    }
    let blocks: Vec<serde_json::Value> = serde_json::from_str(&raw).map_err(|error| {
        ConversionError::Refused(format!("the stored blob is not valid JSON: {error}"))
    })?;
    if expected.is_some_and(|revision| !revision.matches(&blocks)) {
        return Err(ConversionError::Refused(
            "the conversation changed since it was loaded; reopen it before converting".into(),
        ));
    }
    // Render every block's stored form before writing anything, so a refusal
    // (for example, a block carrying its own `rows` member) refuses the
    // whole session up front — never one block, never partially.
    let mut updates = Vec::with_capacity(blocks.len());
    for (seq, block) in blocks.iter().enumerate() {
        let raw = serde_json::to_string(block)
            .map_err(|error| ConversionError::Refused(format!("block {seq}: {error}")))?;
        let rows = blocks::project(std::slice::from_ref(block))
            .map_err(|reason| ConversionError::Refused(format!("block {seq}: {reason}")))?;
        let form = store_block_form(block, &raw, &rows).map_err(|reason| {
            let id = block["id"].as_str().unwrap_or("no id");
            ConversionError::Refused(format!("block {seq} ({id}): {reason}"))
        })?;
        updates.push(BlockUpdate {
            skeleton: form.skeleton,
            rows: form.elements,
            payloads: form.payloads,
        });
    }
    // Only the attempt that still owns this source snapshot may replace the
    // recovery point. A late failed attempt must not publish an older backup.
    if let Some(backup) = backup {
        backup.publish()?;
    }
    // A conversion starts a fresh revision range. Reusing zero would let a
    // stale native handle match again after downgrade/reconversion (ABA).
    // Keep 62 random bits and ample headroom for subsequent increments.
    let initial_revision = (rand::random::<u64>() & ((1_u64 << 62) - 1)) as i64;
    conn.execute(
        "INSERT INTO native_sessions(session_id,mode_version,session_revision) VALUES(?1,?2,?3)",
        rusqlite::params![
            session_id,
            i64::from(super::INITIAL_NATIVE_CHUNKS_MODE_VERSION),
            initial_revision
        ],
    )?;
    #[cfg(test)]
    {
        if fail_if_injected(ConversionStage::Marker) {
            return Err(ConversionError::Interrupted(ConversionStage::Marker));
        }
    }
    let flush = replace_blocks_tx(conn, session_id, initial_revision, &updates)
        .map_err(|error| ConversionError::Writer(error.to_string()))?;
    Ok(ConversionReport {
        blocks: flush.blocks_written,
        chunks: flush.chunks_written,
        bytes_written: flush.chunk_bytes_written,
        session_revision: flush.session_revision,
        mode_version: flush.mode_version,
    })
}

/// Rebuild `blocks_json` from the latest native content and switch the session
/// back to the blob, in one transaction: restart equality is verified first
/// (the rebuilt blob must project to exactly the rows the backend holds, and
/// every block must re-store to its exact stored form), then the blob is
/// written and the native copy — marker, blocks, chunks and payload segments
/// — is deleted. A native copy left behind a Blob marker is a resurrection
/// hazard if the marker ever flips again, so none survives. Any failure rolls
/// everything back: the native content stays valid and the export can be
/// retried once its cause is fixed.
pub(crate) fn export_downgrade_to_blob(
    conn: &mut Connection,
    session_id: &str,
    expected_revision: Option<i64>,
) -> Result<HistoryRevision, String> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let snapshot = downgrade_tx(&tx, session_id, expected_revision)?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(snapshot)
}

fn downgrade_tx(
    conn: &Connection,
    session_id: &str,
    expected_revision: Option<i64>,
) -> Result<HistoryRevision, String> {
    // Another handle may already have exported and removed the native copy.
    // Check after acquiring the write lock, before interpreting native rows.
    if matches!(
        session_storage_mode(conn, session_id).map_err(|error| error.to_string())?,
        StorageMode::Blob
    ) {
        return Err(
            "This conversation has no chunked-storage marker; there is nothing to downgrade".into(),
        );
    }
    let revision = super::session_revision(conn, session_id).map_err(|error| error.to_string())?;
    if expected_revision.is_some_and(|expected| revision != Some(expected)) {
        return Err("This conversation changed in another window. Reopen it before downgrading; its data was preserved.".into());
    }
    let mut blocks = Vec::new();
    for block in iter_full_history(conn, session_id) {
        blocks.push(block?);
    }
    // Restart equality, part one: the blob this export writes must load, in
    // any client, to exactly the rows the chunked backend holds now.
    let projected = blocks::project(&blocks)?;
    let stored_rows = page_rows(conn, session_id, 0, 1 << 40)?;
    if projected != stored_rows {
        return Err(format!(
            "Restart-equality verification failed: rebuilding this conversation's blob projects \
             {} rows but its stored native content holds {}. The downgrade was refused and the \
             native content is unchanged",
            projected.len(),
            stored_rows.len()
        ));
    }
    // Part two: every block must re-render to its exact stored form, proving
    // the blob loses nothing the backend kept (unknown fields, stripped
    // members, receipts) and would convert back to the same bytes.
    let index = super::writer::block_index(conn, session_id).map_err(|error| error.to_string())?;
    for (stored, block) in index.iter().zip(&blocks) {
        let raw = serde_json::to_string(block).map_err(|error| error.to_string())?;
        let rows = blocks::project(std::slice::from_ref(block))?;
        let form = store_block_form(block, &raw, &rows)?;
        let (skeleton, chunks, payloads) =
            super::writer::stored_payload(conn, session_id, stored.block_seq)
                .map_err(|error| error.to_string())?;
        let same_form = |form: &StoredForm| {
            form.skeleton == skeleton
                && pack_stored_chunks(&form.elements, CHUNK_TARGET_BYTES) == chunks
                && form.payloads.len() == payloads.len()
                && form
                    .payloads
                    .iter()
                    .zip(&payloads)
                    .all(|(payload, stored)| {
                        pack_payload_segments(payload, CHUNK_TARGET_BYTES).concat() == *stored
                    })
        };
        // Mode 1 already supports placeholders at arbitrary JSON locations.
        // A block written before expanded stripping must still export without
        // a normalization save; require an exact canonical match in either form.
        let matches_legacy_goal = || -> Result<bool, String> {
            if block["role"] != "goal" {
                return Ok(false);
            }
            // Older writers projected the same goal block as system rows.
            // Verify the exact stored bytes using that projection as well.
            let mut legacy = rows.clone();
            for row in legacy.iter_mut().take_while(|row| row.role == "goal") {
                row.role = "system".into();
            }
            Ok(same_form(&store_block_form(block, &raw, &legacy)?)
                || same_form(&legacy_store_block_form(block, &raw, &legacy)?))
        };
        if !same_form(&form)
            && !matches_legacy_goal()?
            && !same_form(&legacy_store_block_form(block, &raw, &rows)?)
        {
            return Err(format!(
                "Restart-equality verification failed: block {} of this conversation would not \
                 re-store to its stored form. The downgrade was refused and the native content \
                 is unchanged",
                stored.block_seq
            ));
        }
    }
    let encoded = serde_json::to_string(&blocks).map_err(|error| error.to_string())?;
    let written = conn
        .execute(
            "UPDATE sessions SET blocks_json=?2 WHERE id=?1",
            rusqlite::params![session_id, encoded],
        )
        .map_err(|error| error.to_string())?;
    if written == 0 {
        return Err("This conversation does not exist in the desktop schema".into());
    }
    // The blob path's session-list flags, recomputed exactly as a blob save
    // would — present columns only, matching `shared_field`'s fallbacks.
    let mut updates = Vec::new();
    if column_exists(conn, "has_user_message")? {
        updates.push(format!(
            "has_user_message={}",
            i64::from(blocks.iter().any(|block| block["role"] == "user"))
        ));
    }
    if column_exists(conn, "is_draft")? {
        updates.push(format!(
            "is_draft={}",
            i64::from(
                blocks
                    .iter()
                    .any(|block| block["role"] == "user" && block["draft"] == true)
            )
        ));
    }
    if !updates.is_empty() {
        conn.execute(
            &format!("UPDATE sessions SET {} WHERE id=?1", updates.join(",")),
            rusqlite::params![session_id],
        )
        .map_err(|error| error.to_string())?;
    }
    // The native copy goes in the same transaction that restores the blob.
    for table in [
        "native_payloads",
        "native_chunks",
        "native_blocks",
        "native_sessions",
    ] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE session_id=?1"),
            rusqlite::params![session_id],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(HistoryRevision::of(&blocks))
}

/// Whether the sessions table has one column.
fn column_exists(conn: &Connection, column: &str) -> Result<bool, String> {
    let mut statement = conn
        .prepare("PRAGMA table_info(sessions)")
        .map_err(|error| error.to_string())?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(columns.iter().any(|name| name == column))
}

/// Whether the session is an indexed Mono history: those have their own
/// ownership and window rules and are never ordinary blob sessions.
fn indexed_history(conn: &Connection, session_id: &str) -> rusqlite::Result<bool> {
    let has_table: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='mono_transcripts')",
        [],
        |row| row.get(0),
    )?;
    if !has_table {
        return Ok(false);
    }
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM mono_transcripts WHERE session_id=?1)",
        [session_id],
        |row| row.get(0),
    )
}

/// A complete, verified snapshot awaiting publication by its conversion.
#[derive(Debug)]
struct ConsistencyBackup {
    pending: tempfile::NamedTempFile,
    backup: PathBuf,
}

impl ConsistencyBackup {
    fn publish(self) -> Result<(), ConversionError> {
        let Self { pending, backup } = self;
        let backup_error =
            |reason: String| ConversionError::Backup(format!("{reason} at {}", backup.display()));
        // An externally opened recovery database must be closed before its
        // standalone snapshot can safely replace this name.
        for sidecar in [
            backup.with_extension("backup-wal"),
            backup.with_extension("backup-shm"),
        ] {
            if sidecar.exists() {
                return Err(backup_error(
                    "the retained backup has SQLite sidecars; close it before retrying".into(),
                ));
            }
        }
        let published = pending
            .persist(&backup)
            .map_err(|error| backup_error(error.to_string()))?;
        published
            .sync_all()
            .map_err(|error| backup_error(error.to_string()))?;
        #[cfg(unix)]
        std::fs::File::open(
            backup
                .parent()
                .ok_or_else(|| backup_error("no backup directory".into()))?,
        )
        .and_then(|directory| directory.sync_all())
        .map_err(|error| backup_error(error.to_string()))?;
        Ok(())
    }
}

/// A consistent backup of the whole database, beside the database file, taken
/// through SQLite's online backup API from the committed state — never a
/// filesystem copy of a live WAL database. One verified backup per session is
/// retained as `<database>.pre-native-<session-handle>.backup`. Each attempt is
/// built in a separate temporary file beside it; only a closed, verified and
/// synced snapshot replaces the previous recovery point, atomically. A failed
/// copy or verification never deletes or overwrites the retained backup.
fn take_consistency_backup(
    conn: &Connection,
    session_id: &str,
    committed_blob: &str,
) -> Result<Option<ConsistencyBackup>, ConversionError> {
    let Some(database) = main_database_path(conn)? else {
        // An in-memory or temporary database has nothing durable to protect;
        // the transaction below is still fully atomic.
        return Ok(None);
    };
    let backup = PathBuf::from(format!(
        "{}.pre-native-{}.backup",
        database.display(),
        handle(session_id)
    ));
    let backup_error =
        |reason: String| ConversionError::Backup(format!("{reason} at {}", backup.display()));
    // Never attach old WAL sidecars to the new snapshot at publication.
    // An externally opened recovery database must be closed first.
    for sidecar in [
        backup.with_extension("backup-wal"),
        backup.with_extension("backup-shm"),
    ] {
        if sidecar.exists() {
            return Err(backup_error(
                "the retained backup has SQLite sidecars; close it before retrying".into(),
            ));
        }
    }
    let parent = database
        .parent()
        .ok_or_else(|| backup_error("no database directory".into()))?;
    let pending = tempfile::Builder::new()
        .prefix(".native-backup-")
        .tempfile_in(parent)
        .map_err(|error| backup_error(error.to_string()))?;
    let mut destination =
        Connection::open(pending.path()).map_err(|error| backup_error(error.to_string()))?;
    {
        use rusqlite::backup::Backup;
        let copy =
            Backup::new(conn, &mut destination).map_err(|error| backup_error(error.to_string()))?;
        copy.run_to_completion(BACKUP_PAGES_PER_STEP, BACKUP_BUSY_PAUSE, None)
            .map_err(|error| backup_error(error.to_string()))?;
    }
    // Recoverability is verified, not assumed: the backup must hold exactly
    // the committed state the conversion is about to shadow.
    let backed_up: Option<String> = destination
        .query_row(
            "SELECT blocks_json FROM sessions WHERE id=?1",
            [session_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| backup_error(error.to_string()))?;
    if backed_up.as_deref() != Some(committed_blob) {
        return Err(backup_error(
            "the backup does not hold this conversation's committed blob".into(),
        ));
    }
    let integrity: String = destination
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| backup_error(error.to_string()))?;
    if integrity != "ok" {
        return Err(backup_error(
            "the backup failed SQLite integrity verification".into(),
        ));
    }
    // Publish a standalone database, with no WAL whose name could be left
    // behind by the rename. This changes only the backup's journal mode.
    destination
        .pragma_update(None, "journal_mode", "DELETE")
        .map_err(|error| backup_error(error.to_string()))?;
    destination
        .close()
        .map_err(|(_, error)| backup_error(error.to_string()))?;
    pending
        .as_file()
        .sync_all()
        .map_err(|error| backup_error(error.to_string()))?;
    #[cfg(test)]
    {
        let hook = AFTER_BACKUP_COPY.with(|hook| hook.borrow_mut().take());
        if let Some(hook) = hook {
            hook();
        }
    }
    Ok(Some(ConsistencyBackup { pending, backup }))
}

/// The main database's file path, or None for in-memory and temporary
/// databases.
fn main_database_path(conn: &Connection) -> Result<Option<PathBuf>, ConversionError> {
    let mut statement = conn.prepare("PRAGMA database_list")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .find(|(name, _)| name == "main")
        .and_then(|(_, file)| file)
        .filter(|file| !file.is_empty() && file != ":memory:")
        .map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn review_downgrade_rechecks_mode_inside_the_transaction() {
        let store = Store::open_monocode(std::path::Path::new(":memory:")).unwrap();
        let raw = r#"[{"id":"p","role":"user","text":"Keep"}]"#;
        store.conn.execute(
            "INSERT INTO sessions(id,cwd,harness,model,runtime_mode,title,blocks_json,created_at,updated_at)
             VALUES('plain','synthetic','codex','synthetic','supervised','Synthetic',?1,1,1)", [raw],
        ).unwrap();
        let tx = store.conn.unchecked_transaction().unwrap();
        // Another window has already downgraded before this transaction began.
        assert!(downgrade_tx(&tx, "plain", None).is_err());
        let saved: String = tx
            .query_row(
                "SELECT blocks_json FROM sessions WHERE id='plain'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(saved, raw);
    }

    #[test]
    fn review_late_backup_cannot_replace_a_newer_conversion_recovery_point() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup-race.db");
        let mut a = Store::open_monocode(&path).unwrap();
        let first = r#"[{"id":"p","role":"user","text":"First"}]"#;
        let latest = r#"[{"id":"p","role":"user","text":"Latest"}]"#;
        a.conn.execute(
            "INSERT INTO sessions(id,cwd,harness,model,runtime_mode,title,blocks_json,created_at,updated_at)
             VALUES('recovery','synthetic','codex','synthetic','supervised','Synthetic',?1,1,1)", [first],
        ).unwrap();
        let id = a.resolve_session_id("recovery").unwrap();
        let other_path = path.clone();
        AFTER_BACKUP_COPY.set(Some(Box::new(move || {
            let mut b = Store::open_monocode(&other_path).unwrap();
            b.conn
                .execute(
                    "UPDATE sessions SET blocks_json=?1 WHERE id='recovery'",
                    [latest],
                )
                .unwrap();
            let other = b.resolve_session_id("recovery").unwrap();
            b.convert_session_to_chunked(other).unwrap();
        })));
        assert!(a.convert_session_to_chunked(id).is_err());
        let backup = PathBuf::from(format!(
            "{}.pre-native-{}.backup",
            path.display(),
            handle("recovery")
        ));
        let saved: String = Connection::open(&backup)
            .unwrap()
            .query_row(
                "SELECT blocks_json FROM sessions WHERE id='recovery'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            saved, latest,
            "a late old snapshot must not replace the winning conversion's backup"
        );
    }

    #[test]
    fn review_failed_backup_preserves_previous_verified_recovery_point() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup.db");
        let store = Store::open_monocode(&path).unwrap();
        let first = r#"[{"id":"p","role":"user","text":"First"}]"#;
        let latest = r#"[{"id":"p","role":"user","text":"Latest"}]"#;
        store.conn.execute(
            "INSERT INTO sessions(id,cwd,harness,model,runtime_mode,title,blocks_json,created_at,updated_at)
             VALUES('recovery','synthetic','codex','synthetic','supervised','Synthetic',?1,1,1)",
            [first],
        ).unwrap();
        take_consistency_backup(&store.conn, "recovery", first)
            .unwrap()
            .unwrap()
            .publish()
            .unwrap();
        let backup = PathBuf::from(format!(
            "{}.pre-native-{}.backup",
            path.display(),
            handle("recovery")
        ));
        let saved_blob = || {
            let conn = Connection::open(&backup).unwrap();
            conn.query_row(
                "SELECT blocks_json FROM sessions WHERE id='recovery'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        };
        assert_eq!(saved_blob(), first);
        store
            .conn
            .execute(
                "UPDATE sessions SET blocks_json=?1 WHERE id='recovery'",
                [latest],
            )
            .unwrap();
        // A concurrent change makes verification fail. The unverified copy
        // must not replace the previous verified recovery point.
        let error = take_consistency_backup(&store.conn, "recovery", first).unwrap_err();
        assert!(matches!(error, ConversionError::Backup(_)), "{error}");
        assert_eq!(saved_blob(), first);
        take_consistency_backup(&store.conn, "recovery", latest)
            .unwrap()
            .unwrap()
            .publish()
            .unwrap();
        assert_eq!(saved_blob(), latest);
        let retained = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".backup"))
            .count();
        assert_eq!(retained, 1, "only the latest verified backup is retained");
    }
}
