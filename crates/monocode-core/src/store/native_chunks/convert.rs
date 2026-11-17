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
use super::layout::{pack_payload_segments, pack_stored_chunks, CHUNK_TARGET_BYTES};
use super::writer::{replace_blocks_tx, BlockUpdate};
use super::{
    iter_full_history, marker_mode_version, page_rows, session_storage_mode, store_block_form,
    StorageMode,
};
use crate::store::shared::blocks;
use crate::store::shared::handle;
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
    /// A block refuses chunked storage (a reserved marker in its JSON, or its
    /// own `rows` member). The whole session is refused, never one block.
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
) -> Result<ConversionReport, ConversionError> {
    // Refuse before touching anything: the session must be an ordinary blob.
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
    take_consistency_backup(conn, session_id, &committed)?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    match convert_tx(conn, session_id, &committed) {
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
    if raw != backed_up {
        return Err(ConversionError::Backup(
            "the conversation changed while its backup was taken; retry the conversion".into(),
        ));
    }
    let blocks: Vec<serde_json::Value> = serde_json::from_str(&raw).map_err(|error| {
        ConversionError::Refused(format!("the stored blob is not valid JSON: {error}"))
    })?;
    // Render every block's stored form before writing anything, so a refusal
    // (a reserved marker, a block carrying its own `rows` member) refuses the
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
    // The marker lands first, at revision 0, and the blocks go through the
    // writer's own full-replace path — the canonical packing the anchored
    // rewrite on every later save assumes, never a hand-rolled chunk layout.
    conn.execute(
        "INSERT INTO native_sessions(session_id,mode_version,session_revision) VALUES(?1,?2,0)",
        rusqlite::params![session_id, i64::from(super::NATIVE_CHUNKS_MODE_VERSION)],
    )?;
    #[cfg(test)]
    {
        if fail_if_injected(ConversionStage::Marker) {
            return Err(ConversionError::Interrupted(ConversionStage::Marker));
        }
    }
    let flush = replace_blocks_tx(conn, session_id, 0, &updates)
        .map_err(|error| ConversionError::Writer(error.to_string()))?;
    Ok(ConversionReport {
        blocks: flush.blocks_written,
        chunks: flush.chunks_written,
        bytes_written: flush.chunk_bytes_written,
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
) -> Result<(), String> {
    if matches!(
        session_storage_mode(conn, session_id).map_err(|error| error.to_string())?,
        StorageMode::Blob
    ) {
        return Err(
            "This conversation has no chunked-storage marker; there is nothing to downgrade".into(),
        );
    }
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| error.to_string())?;
    match downgrade_tx(conn, session_id) {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .map_err(|error| error.to_string()),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn downgrade_tx(conn: &Connection, session_id: &str) -> Result<(), String> {
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
        let same = form.skeleton == skeleton
            && pack_stored_chunks(&form.elements, CHUNK_TARGET_BYTES) == chunks
            && form.payloads.len() == payloads.len()
            && form
                .payloads
                .iter()
                .zip(&payloads)
                .all(|(payload, stored)| {
                    pack_payload_segments(payload, CHUNK_TARGET_BYTES).concat() == *stored
                });
        if !same {
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
    Ok(())
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

/// A consistent backup of the whole database, beside the database file, taken
/// through SQLite's online backup API from the committed state — never a
/// filesystem copy of a live WAL database. The deterministic name is per
/// session, so a retried conversion overwrites its own backup and concurrent
/// conversions of different sessions keep one each. There is no app-wide
/// backup location today (the store layer keeps none), so this documents the
/// convention it installs: `<database>.pre-native-<session-handle>.backup`.
/// The backup is kept after a successful conversion — it is the recovery
/// point, not a temporary.
fn take_consistency_backup(
    conn: &Connection,
    session_id: &str,
    committed_blob: &str,
) -> Result<(), ConversionError> {
    let Some(database) = main_database_path(conn)? else {
        // An in-memory or temporary database has nothing durable to protect;
        // the transaction below is still fully atomic.
        return Ok(());
    };
    let backup = PathBuf::from(format!(
        "{}.pre-native-{}.backup",
        database.display(),
        handle(session_id)
    ));
    let backup_error =
        |reason: String| ConversionError::Backup(format!("{reason} at {}", backup.display()));
    // A leftover backup from an earlier attempt (or its sidecar files) is
    // removed first, so every attempt writes a complete fresh copy.
    for stale in [
        backup.with_extension("backup-wal"),
        backup.with_extension("backup-shm"),
        backup.clone(),
    ] {
        let _ = std::fs::remove_file(stale);
    }
    let mut destination =
        Connection::open(&backup).map_err(|error| backup_error(error.to_string()))?;
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
    Ok(())
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
