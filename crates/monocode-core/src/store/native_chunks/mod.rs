//! The chunked storage backend for ordinary Native local sessions (batch 2).
//!
//! The backend lives in the same main SQLite database as the desktop schema:
//! `native_sessions` holds one storage-mode marker per session — an absent
//! marker means the session is still a desktop `blocks_json` blob — while
//! `native_blocks`, `native_chunks` and `native_payloads` hold each block's
//! skeleton, its row elements in byte-bounded chunks, and the oversized
//! members stripped from the skeleton (`layout` owns the tables and that
//! raw-JSON representation). This module owns the mode marker, the open-time
//! write barrier that keeps an older build from writing a database whose
//! sessions use a newer storage mode, the readers (`reader`: full-history
//! block iteration and windowed row pagination over the stored absolute row
//! numbers), the sparse block-level writer (`writer`) that saves only the
//! blocks a save actually changed, and the per-session conversion between the
//! two formats (`convert`).
mod convert;
mod layout;
pub(crate) mod reader;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod writer;

pub use convert::ConversionReport;
pub(crate) use convert::{convert_session_to_native, export_downgrade_to_blob};
#[cfg(test)]
pub(crate) use convert::{fail_next_conversion_at_for_test, ConversionStage};
pub(crate) use layout::{block_value, ensure_native_chunks_tables, store_block_form};
pub(crate) use reader::{iter_full_history, page_rows};
pub use writer::DirtyBlock;
pub(crate) use writer::{
    block_index, flush_sparse_tx, replace_blocks_tx, session_revision, stored_payload, BlockUpdate,
    SparseFlush, SparseSavePlan, StoredBlockIndex,
};

use rusqlite::{params, Connection, OptionalExtension};

/// The newest storage-mode version this build writes and understands. A marker
/// claiming a higher version makes the whole connection refuse writes.
/// Public so out-of-crate callers that create their own sessions (the
/// persistence probe) can opt them into this backend with the version the
/// store itself dispatches on.
pub const NATIVE_CHUNKS_MODE_VERSION: u32 = 2;

/// Plain blocks still use the original format until a full-block payload is needed.
pub(crate) const INITIAL_NATIVE_CHUNKS_MODE_VERSION: u32 = 1;

/// The distinct error kind surfaced when a database uses a storage mode newer
/// than this build understands. An unrecognized format must refuse writes,
/// never read as an empty history (spec: 数据转换及回退).
pub(crate) const UNSUPPORTED_STORAGE_MODE: &str = "UnsupportedStorageMode";

/// How one session's transcript is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StorageMode {
    /// The desktop schema's single `sessions.blocks_json` document.
    Blob,
    /// The chunked backend, at the marker's storage-mode version.
    NativeChunks { mode_version: u32 },
}

/// Read one session's storage mode. An absent marker — or an entirely absent
/// `native_sessions` table, on databases from before this backend — is `Blob`.
pub(crate) fn session_storage_mode(
    conn: &Connection,
    session_id: &str,
) -> rusqlite::Result<StorageMode> {
    let marker = marker_mode_version(conn, session_id)?;
    Ok(match marker {
        None => StorageMode::Blob,
        Some(version) => {
            if version > i64::from(NATIVE_CHUNKS_MODE_VERSION) {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Integer,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        format!(
                            "{UNSUPPORTED_STORAGE_MODE}: this conversation uses storage mode \
                             version {version}, newer than this Monoptah build understands \
                             ({NATIVE_CHUNKS_MODE_VERSION}). Update Monoptah to read it."
                        ),
                    )),
                ));
            }
            StorageMode::NativeChunks {
                mode_version: u32::try_from(version)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, version))?,
            }
        }
    })
}

/// The raw `mode_version` of one session's marker, if the marker exists.
fn marker_mode_version(conn: &Connection, session_id: &str) -> rusqlite::Result<Option<i64>> {
    if !has_native_sessions(conn)? {
        return Ok(None);
    }
    conn.query_row(
        "SELECT mode_version FROM native_sessions WHERE session_id=?1",
        [session_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
}

fn has_native_sessions(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='native_sessions')",
        [],
        |row| row.get(0),
    )
}

/// The open-time write barrier. The newest storage-mode version any marker
/// claims — a per-session marker, or a future whole-database marker, which is
/// just another row in the same table — decides whether this build may write.
/// Returns the refusal message for every write path when some marker is newer
/// than `NATIVE_CHUNKS_MODE_VERSION`. Versions are compared as raw integers so
/// an absurdly large one still refuses instead of wrapping into support.
pub(crate) fn unsupported_storage_mode_refusal(
    conn: &Connection,
) -> rusqlite::Result<Option<String>> {
    if !has_native_sessions(conn)? {
        return Ok(None);
    }
    let newest: Option<i64> =
        conn.query_row("SELECT max(mode_version) FROM native_sessions", [], |row| {
            row.get(0)
        })?;
    if let Some(version) = newest.filter(|version| *version > i64::from(NATIVE_CHUNKS_MODE_VERSION))
    {
        return Ok(Some(format!(
            "{UNSUPPORTED_STORAGE_MODE}: a session in this database is stored with storage \
             mode version {version}, newer than this Monoptah build understands \
             ({NATIVE_CHUNKS_MODE_VERSION}). Update Monoptah before writing to it; sessions \
             with supported storage modes remain readable."
        )));
    }
    Ok(None)
}

/// Write one session's desktop blob, refusing when the session carries a
/// chunked-storage marker. The marker check rides inside the UPDATE itself,
/// so a session another window converted between the caller's mode dispatch
/// and this write refuses instead of forking its history across two backends
/// (the old client's blob beside the chunked backend's copy). This is the
/// blob writer's side of the no-dual-write contract: the refusal carries the
/// distinct `UnsupportedStorageMode` kind, never a silent parallel write.
///
/// The backend's tables are ensured first (idempotent, additive only, the
/// same statement a new database or `prefer_chunked_storage` runs) so the
/// guard's subquery is always evaluable.
pub(crate) fn store_blob_blocks(
    conn: &Connection,
    session_id: &str,
    encoded: &str,
) -> Result<(), String> {
    ensure_native_chunks_tables(conn).map_err(|error| error.to_string())?;
    let written = conn
        .execute(
            "UPDATE sessions SET blocks_json=?2 WHERE id=?1
             AND NOT EXISTS(SELECT 1 FROM native_sessions WHERE session_id=?1)",
            params![session_id, encoded],
        )
        .map_err(|error| error.to_string())?;
    if written == 0 {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
                [session_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if exists {
            return Err(format!(
                "{UNSUPPORTED_STORAGE_MODE}: this conversation is stored in the chunked \
                 backend, so the desktop blob writer refused to write beside it. Reopen the \
                 conversation; its data was preserved."
            ));
        }
        return Err("Session no longer exists".into());
    }
    Ok(())
}
