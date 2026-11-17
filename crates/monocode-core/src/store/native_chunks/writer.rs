//! The sparse block-level writer for chunked-mode sessions.
//!
//! A save renders only the blocks it actually changed (the caller decides
//! dirtiness; `shared`'s dispatch maps row ranges onto stored block extents)
//! and hands them to [`flush_sparse_tx`] as a [`SparseSavePlan`]. The writer
//! opens one `BEGIN IMMEDIATE` transaction, re-reads the session revision and
//! every touched block's placement, refuses on mismatch ([`FlushConflict`];
//! the caller surfaces the refusal — it never auto-reloads, spec L103), bumps
//! the touched blocks' and the session's revisions, and commits. Blocks the
//! plan does not mention are neither read nor written, so unchanged history
//! bytes never re-enter a save.
//!
//! [`replace_blocks_tx`] is the one full-replace entry: the blob-equivalent
//! path for saves that legitimately re-derive a session's whole block list
//! (turn receipts, follow-ups, rewind after an edit, cloud restore). It is
//! the only writer that recomputes absolute row extents; the sparse path
//! never renumbers.
use super::layout::{pack_payload_segments, pack_stored_chunks, ROWS_PLACEHOLDER};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::{collections::HashSet, fmt};

/// The chunk width a block's rows and stripped payload members are packed
/// into; re-exported from [`super::layout`] where the stored form is defined,
/// so existing imports of `writer::CHUNK_TARGET_BYTES` keep resolving.
pub(crate) use super::layout::CHUNK_TARGET_BYTES;

/// One block a save touches: its sequence and the inclusive absolute rows it
/// will hold once the save lands. New rows only ever extend the tail block or
/// start new blocks, so absolute row numbers never shift.
pub struct DirtyBlock {
    pub block_seq: i64,
    pub first_row: i64,
    pub last_row: i64,
}
impl Clone for DirtyBlock {
    fn clone(&self) -> Self {
        Self {
            block_seq: self.block_seq,
            first_row: self.first_row,
            last_row: self.last_row,
        }
    }
}
impl fmt::Debug for DirtyBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirtyBlock")
            .field("block_seq", &self.block_seq)
            .field("first_row", &self.first_row)
            .field("last_row", &self.last_row)
            .finish()
    }
}
impl PartialEq for DirtyBlock {
    fn eq(&self, other: &Self) -> bool {
        self.block_seq == other.block_seq
            && self.first_row == other.first_row
            && self.last_row == other.last_row
    }
}
impl Eq for DirtyBlock {}

/// What one save intends to write: the session revision it was planned
/// against, and the blocks it carries. An empty `dirty` writes nothing.
pub(crate) struct SparseSavePlan {
    pub session_revision_expected: i64,
    pub dirty: Vec<DirtyBlock>,
}

/// Why a flush refused to write: the session moved underneath the plan, or one
/// touched block did. The caller surfaces the refusal and keeps its pending
/// work; it never silently reloads.
#[derive(Debug)]
pub(crate) enum FlushConflict {
    StaleSession,
    StaleBlock { block_seq: i64 },
}
impl fmt::Display for FlushConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleSession => write!(
                f,
                "This conversation changed in another window. Reopen it before saving; its \
                 data was preserved."
            ),
            Self::StaleBlock { block_seq } => write!(
                f,
                "This conversation changed in another window (block {block_seq} moved). Reopen \
                 it before saving; its data was preserved."
            ),
        }
    }
}

/// One block's stored form, as the render callback produces it: the skeleton
/// (raw JSON with the rows placeholder, giant members stripped) and the
/// serialized row elements, plus the serialized values of the members moved
/// into `native_payloads`, in the order their placeholders appear in the
/// skeleton.
pub(crate) struct BlockUpdate {
    pub skeleton: String,
    pub rows: Vec<String>,
    pub payloads: Vec<String>,
}
impl Clone for BlockUpdate {
    fn clone(&self) -> Self {
        Self {
            skeleton: self.skeleton.clone(),
            rows: self.rows.clone(),
            payloads: self.payloads.clone(),
        }
    }
}

/// What one sparse flush did: the session revision after the commit, and the
/// block/chunk write counters `SaveSample` records and the probe's v3 samples
/// report. `chunks_written` and `chunk_bytes_written` count only the chunks
/// (and payload segments) whose bytes were written. `skeleton_bytes_written`
/// counts the skeleton JSON supplied to block upserts, including unchanged
/// skeletons submitted again with a changed block. Their sum is transcript JSON
/// bytes, excluding scalar metadata, indexes and SQLite page/WAL overhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SparseFlush {
    pub session_revision: i64,
    pub blocks_written: u64,
    pub chunks_written: u64,
    pub chunk_bytes_written: u64,
    pub skeleton_bytes_written: u64,
}

/// Every way a sparse flush refuses or fails. Distinct refusals — unknown
/// blocks, missing or duplicate block ids — are never silently dropped.
#[derive(Debug)]
pub(crate) enum FlushError {
    Conflict(FlushConflict),
    /// The plan's session has no chunked-storage marker.
    NotChunked,
    /// The plan names a block_seq that is neither stored nor the next append.
    UnknownBlock {
        block_seq: i64,
    },
    /// The plan lists one block_seq twice.
    DuplicateBlock {
        block_seq: i64,
    },
    /// An appended block's skeleton carries no block id.
    MissingBlockId {
        block_seq: i64,
    },
    /// Two blocks in the write set claim one block id.
    DuplicateBlockId {
        block_id: String,
    },
    /// Rendered content for a stored block carries a different id.
    BlockIdMismatch {
        block_seq: i64,
    },
    /// The render callback failed, or its rows disagree with the plan.
    Render(String),
    Sqlite(rusqlite::Error),
}
impl From<rusqlite::Error> for FlushError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}
impl fmt::Display for FlushError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(conflict) => write!(f, "{conflict}"),
            Self::NotChunked => write!(
                f,
                "This session has no chunked-storage marker; the sparse writer refused it"
            ),
            Self::UnknownBlock { block_seq } => write!(
                f,
                "The sparse plan references block_seq {block_seq}, which is neither a stored \
                 block nor the next append at the tail; the save was refused"
            ),
            Self::DuplicateBlock { block_seq } => write!(
                f,
                "The sparse plan lists block_seq {block_seq} twice; the save was refused"
            ),
            Self::MissingBlockId { block_seq } => write!(
                f,
                "A new block at block_seq {block_seq} has no id; the save was refused"
            ),
            Self::DuplicateBlockId { block_id } => write!(
                f,
                "Two blocks claim the id \"{block_id}\"; the save was refused"
            ),
            Self::BlockIdMismatch { block_seq } => write!(
                f,
                "Rendered content for block_seq {block_seq} carries a different id than the \
                 stored block; the save was refused"
            ),
            Self::Render(reason) => write!(f, "{reason}"),
            Self::Sqlite(error) => write!(f, "{error}"),
        }
    }
}

/// One stored block's placement in the session's absolute row numbering.
pub(crate) struct StoredBlockIndex {
    pub block_seq: i64,
    pub block_id: String,
    pub first_row: i64,
    pub row_count: i64,
    pub block_revision: i64,
}

/// The session's stored blocks in block order: the sparse plan's map of row
/// ranges onto blocks. Payloads (skeletons, chunks) are not read here.
pub(crate) fn block_index(
    conn: &Connection,
    session_id: &str,
) -> rusqlite::Result<Vec<StoredBlockIndex>> {
    super::session_storage_mode(conn, session_id)?;
    let mut stmt = conn.prepare(
        "SELECT block_seq,block_id,first_row,row_count,block_revision
         FROM native_blocks WHERE session_id=?1 ORDER BY block_seq",
    )?;
    let index = stmt
        .query_map(params![session_id], |row| {
            Ok(StoredBlockIndex {
                block_seq: row.get(0)?,
                block_id: row.get(1)?,
                first_row: row.get(2)?,
                row_count: row.get(3)?,
                block_revision: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(index)
}

/// The session's current chunked-storage revision, if a marker exists. A
/// database without the marker table (every pre-batch-2 database) has none,
/// so loads there stay Blob writes instead of failing on a missing table.
pub(crate) fn session_revision(
    conn: &Connection,
    session_id: &str,
) -> rusqlite::Result<Option<i64>> {
    if !super::has_native_sessions(conn)? {
        return Ok(None);
    }
    super::session_storage_mode(conn, session_id)?;
    conn.query_row(
        "SELECT session_revision FROM native_sessions WHERE session_id=?1",
        [session_id],
        |row| row.get(0),
    )
    .optional()
}

/// The standalone sparse flush: one `BEGIN IMMEDIATE` transaction around
/// [`flush_sparse_tx`]. The shared store composes `flush_sparse_tx` with the
/// queue dequeue and delivery receipt in its own transaction instead; tests
/// and later tasks drive this entry directly.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn flush_sparse(
    conn: &mut Connection,
    session_id: &str,
    plan: &SparseSavePlan,
    render: &dyn Fn(&DirtyBlock) -> Result<BlockUpdate, String>,
) -> Result<SparseFlush, FlushError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(FlushError::Sqlite)?;
    match flush_sparse_tx(&tx, session_id, plan, render) {
        Ok(flush) => tx.commit().map_err(FlushError::Sqlite).map(|()| flush),
        Err(error) => {
            let _ = tx.rollback();
            Err(error)
        }
    }
}

/// The sparse flush inside a caller-owned transaction (already under
/// `BEGIN IMMEDIATE`): re-read the session revision and every touched block's
/// placement, refuse on mismatch, write only blocks whose stored bytes change,
/// then bump the touched blocks' and the session's revisions.
pub(crate) fn flush_sparse_tx(
    conn: &Connection,
    session_id: &str,
    plan: &SparseSavePlan,
    render: &dyn Fn(&DirtyBlock) -> Result<BlockUpdate, String>,
) -> Result<SparseFlush, FlushError> {
    let Some(stored_revision) = session_revision(conn, session_id)? else {
        return Err(FlushError::NotChunked);
    };
    if stored_revision != plan.session_revision_expected {
        return Err(FlushError::Conflict(FlushConflict::StaleSession));
    }
    let mut dirty = plan.dirty.clone();
    dirty.sort_unstable_by_key(|block| block.block_seq);
    for pair in dirty.windows(2) {
        if pair[0].block_seq == pair[1].block_seq {
            return Err(FlushError::DuplicateBlock {
                block_seq: pair[1].block_seq,
            });
        }
    }
    let index = block_index(conn, session_id)?;
    let total_rows = index
        .last()
        .map_or(0, |block| block.first_row + block.row_count);
    let max_seq = index.last().map_or(-1, |block| block.block_seq);
    let mut claimed_ids: HashSet<String> = index.iter().map(|b| b.block_id.clone()).collect();
    let mut stats = SparseFlush {
        session_revision: stored_revision,
        blocks_written: 0,
        chunks_written: 0,
        chunk_bytes_written: 0,
        skeleton_bytes_written: 0,
    };
    // Where the next appended block may land; tail growth moves this forward.
    let mut tail_extent = total_rows;
    let mut next_append_seq = max_seq + 1;
    for block in &dirty {
        let update = render(block).map_err(FlushError::Render)?;
        let planned = block.last_row - block.first_row + 1;
        if i64::try_from(update.rows.len()) != Ok(planned) {
            return Err(FlushError::Render(format!(
                "Rendered {} rows for block_seq {}, but its plan covers {planned}",
                update.rows.len(),
                block.block_seq
            )));
        }
        let Some(stored) = index
            .iter()
            .find(|stored| stored.block_seq == block.block_seq)
        else {
            // A block the database never stored may only be the next append.
            if block.block_seq != next_append_seq || block.first_row != tail_extent {
                return Err(FlushError::UnknownBlock {
                    block_seq: block.block_seq,
                });
            }
            let block_id = skeleton_id(&update.skeleton).ok_or(FlushError::MissingBlockId {
                block_seq: block.block_seq,
            })?;
            if !claimed_ids.insert(block_id.clone()) {
                return Err(FlushError::DuplicateBlockId { block_id });
            }
            write_block(
                conn,
                session_id,
                &BlockPlacement {
                    block_seq: block.block_seq,
                    block_id,
                    first_row: block.first_row,
                    row_count: planned,
                    block_revision: 0,
                },
                &update,
                &mut stats,
            )?;
            tail_extent += planned;
            next_append_seq = block.block_seq + 1;
            continue;
        };
        // The block must sit exactly where the plan placed it; anything else
        // means another writer moved rows underneath.
        if stored.first_row != block.first_row
            || block.last_row < stored.first_row + stored.row_count - 1
        {
            return Err(FlushError::Conflict(FlushConflict::StaleBlock {
                block_seq: block.block_seq,
            }));
        }
        let rendered_id = skeleton_id(&update.skeleton);
        if rendered_id.as_deref() != Some(stored.block_id.as_str()) {
            return Err(FlushError::BlockIdMismatch {
                block_seq: block.block_seq,
            });
        }
        let is_last = stored.block_seq == max_seq;
        let new_count = update.rows.len() as i64;
        if new_count < stored.row_count || (new_count > stored.row_count && !is_last) {
            return Err(FlushError::Render(format!(
                "Block {} would change from {} to {new_count} rows, which renumbers later \
                 blocks; the save was refused",
                stored.block_seq, stored.row_count
            )));
        }
        // Same-value acceptance: identical stored form writes nothing.
        if block_matches_stored(conn, session_id, stored.block_seq, &update)? {
            continue;
        }
        write_block(
            conn,
            session_id,
            &BlockPlacement {
                block_seq: stored.block_seq,
                block_id: stored.block_id.clone(),
                first_row: stored.first_row,
                row_count: new_count,
                block_revision: stored.block_revision + 1,
            },
            &update,
            &mut stats,
        )?;
        if is_last {
            tail_extent = stored.first_row + new_count;
        }
    }
    if stats.blocks_written > 0 {
        conn.execute(
            "UPDATE native_sessions SET session_revision=?2 WHERE session_id=?1",
            params![session_id, stored_revision + 1],
        )?;
        stats.session_revision = stored_revision + 1;
    }
    Ok(stats)
}

/// The full-replace flush inside a caller-owned transaction: store exactly
/// `blocks` as the session's block list, the blob-equivalent of rewriting
/// `blocks_json`. Blocks past the new list are deleted (their chunks and
/// payload segments with them); every kept block keeps its id and content but
/// its absolute extent is recomputed sequentially, so a truncated history
/// renumbers later blocks exactly as the blob path's re-derivation would.
/// Blocks whose stored bytes are unchanged are skipped, so a turn receipt or
/// follow-up that touches one block rewrites only that block.
pub(crate) fn replace_blocks_tx(
    conn: &Connection,
    session_id: &str,
    expected_revision: i64,
    blocks: &[BlockUpdate],
) -> Result<SparseFlush, FlushError> {
    let Some(stored_revision) = session_revision(conn, session_id)? else {
        return Err(FlushError::NotChunked);
    };
    if stored_revision != expected_revision {
        return Err(FlushError::Conflict(FlushConflict::StaleSession));
    }
    let index = block_index(conn, session_id)?;
    let mut claimed_ids: HashSet<String> = HashSet::new();
    let mut stats = SparseFlush {
        session_revision: stored_revision,
        blocks_written: 0,
        chunks_written: 0,
        chunk_bytes_written: 0,
        skeleton_bytes_written: 0,
    };
    let mut deleted = false;
    for stored in &index {
        if stored.block_seq >= blocks.len() as i64 {
            delete_block_rows(conn, session_id, stored.block_seq)?;
            deleted = true;
        }
    }
    let mut first_row = 0i64;
    for (seq, update) in blocks.iter().enumerate() {
        let block_seq = seq as i64;
        let block_id =
            skeleton_id(&update.skeleton).ok_or(FlushError::MissingBlockId { block_seq })?;
        if !claimed_ids.insert(block_id.clone()) {
            return Err(FlushError::DuplicateBlockId { block_id });
        }
        let row_count = update.rows.len() as i64;
        if let Some(stored) = index.iter().find(|stored| stored.block_seq == block_seq) {
            if stored.block_id != block_id {
                return Err(FlushError::BlockIdMismatch { block_seq });
            }
            // Same skeleton, chunks, payloads and extent: nothing to write.
            if stored.first_row == first_row
                && stored.row_count == row_count
                && block_matches_stored(conn, session_id, block_seq, update)?
            {
                first_row += row_count;
                continue;
            }
            write_block(
                conn,
                session_id,
                &BlockPlacement {
                    block_seq,
                    block_id,
                    first_row,
                    row_count,
                    block_revision: stored.block_revision + 1,
                },
                update,
                &mut stats,
            )?;
        } else {
            write_block(
                conn,
                session_id,
                &BlockPlacement {
                    block_seq,
                    block_id,
                    first_row,
                    row_count,
                    block_revision: 0,
                },
                update,
                &mut stats,
            )?;
        }
        first_row += row_count;
    }
    if stats.blocks_written > 0 || deleted {
        conn.execute(
            "UPDATE native_sessions SET session_revision=?2 WHERE session_id=?1",
            params![session_id, stored_revision + 1],
        )?;
        stats.session_revision = stored_revision + 1;
    }
    Ok(stats)
}

/// One bounded chunk write a save plans against a block's stored chunks.
pub(crate) enum ChunkOp {
    /// Replace the stored chunk at `chunk_ord` with `rows_json`.
    Rewrite(i64, String),
    /// Store `rows_json` as the block's next chunk, past the last stored ordinal.
    Append(String),
    /// Delete every stored chunk from `chunk_ord` on.
    Truncate(i64),
}

/// Plan one block's chunk writes: pack the rendered rows canonically
/// ([`pack_stored_chunks`] at [`CHUNK_TARGET_BYTES`]), keep every stored chunk
/// whose bytes already match — anchored from both ends, so a change in the
/// middle leaves the chunks on either side untouched — rewrite the span in
/// between, append past the stored ordinals and truncate orphans.
///
/// The canonical packing makes the writes bounded where the rows are: an
/// append only moves the tail chunk (earlier boundaries are fixed by the
/// unchanged earlier rows), and an edit whose row keeps its serialized width
/// moves only the chunk holding it, with byte-identical chunks on both sides
/// skipped. A row larger than the target is byte-split into bounded pieces
/// (`layout`'s continuation markers), so even a single giant row — a tool
/// block's serialized payload — never becomes one unbounded chunk; an insert
/// inside it rewrites only the pieces the changed bytes moved plus the small
/// tail span, and byte-identical pieces stay anchored. The stored layout is
/// always the canonical packing of the stored rows — no drift accumulates
/// across saves.
/// Anchoring over the reference row packing; the writer anchors the stored
/// packing directly, and the tests pin this composition.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn chunk_rows(rows: &[String], existing: &[(i64, &str)]) -> Vec<ChunkOp> {
    anchor_ops(pack_stored_chunks(rows, CHUNK_TARGET_BYTES), existing)
}

/// The anchored write plan for one ordered list of stored pieces against the
/// pieces already stored (in ordinal order): keep the byte-identical prefix
/// and suffix, rewrite the span between, append past the end, truncate
/// orphans. Used for row chunks and payload segments alike.
fn anchor_ops(packed: Vec<String>, existing: &[(i64, &str)]) -> Vec<ChunkOp> {
    let prefix = existing
        .iter()
        .zip(&packed)
        .take_while(|((_, stored), chunk)| stored == chunk)
        .count()
        .min(packed.len());
    let mut suffix = existing
        .iter()
        .rev()
        .zip(packed.iter().rev())
        .take_while(|((_, stored), chunk)| stored == chunk)
        .count()
        .min(existing.len().saturating_sub(prefix));
    if suffix > 0 && packed.len() != existing.len() {
        // The re-packed middle would need ordinals the anchored suffix holds:
        // rewrite through the tail instead of renumbering verified chunks.
        suffix = 0;
    }
    let mut ops = Vec::new();
    for (index, chunk) in packed
        .iter()
        .enumerate()
        .take(packed.len() - suffix)
        .skip(prefix)
    {
        match existing.get(index) {
            Some((ord, _)) => ops.push(ChunkOp::Rewrite(*ord, chunk.clone())),
            None => ops.push(ChunkOp::Append(chunk.clone())),
        }
    }
    if suffix == 0 && packed.len() < existing.len() {
        ops.push(ChunkOp::Truncate(existing[packed.len()].0));
    }
    ops
}

/// Where one block sits and which revision it carries as the write lands.
struct BlockPlacement {
    block_seq: i64,
    block_id: String,
    first_row: i64,
    row_count: i64,
    block_revision: i64,
}

/// Apply one block's chunk and payload plans and its block row. Chunks and
/// payload segments whose canonical packing is byte-identical to what is
/// stored are not written again, so the counters in `stats` report the moved
/// bytes, not the block's whole payload.
fn write_block(
    conn: &Connection,
    session_id: &str,
    place: &BlockPlacement,
    update: &BlockUpdate,
    stats: &mut SparseFlush,
) -> rusqlite::Result<()> {
    let existing = stored_chunk_pairs(conn, session_id, place.block_seq)?;
    let mut next_ord = existing.last().map_or(0, |(ord, _)| ord + 1);
    let existing_refs: Vec<(i64, &str)> = existing
        .iter()
        .map(|(ord, text)| (*ord, text.as_str()))
        .collect();
    for op in anchor_ops(
        pack_stored_chunks(&update.rows, CHUNK_TARGET_BYTES),
        &existing_refs,
    ) {
        match op {
            ChunkOp::Rewrite(ord, json) => {
                store_chunk(conn, session_id, place.block_seq, ord, &json)?;
                stats.chunks_written += 1;
                stats.chunk_bytes_written += json.len() as u64;
            }
            ChunkOp::Append(json) => {
                store_chunk(conn, session_id, place.block_seq, next_ord, &json)?;
                next_ord += 1;
                stats.chunks_written += 1;
                stats.chunk_bytes_written += json.len() as u64;
            }
            ChunkOp::Truncate(keep) => {
                conn.execute(
                    "DELETE FROM native_chunks WHERE session_id=?1 AND block_seq=?2 AND chunk_ord>=?3",
                    params![session_id, place.block_seq, keep],
                )?;
            }
        }
    }
    write_payload_segments(conn, session_id, place.block_seq, &update.payloads, stats)?;
    conn.execute(
        "INSERT INTO native_blocks(session_id,block_seq,block_id,skeleton_json,first_row,row_count,block_revision)
         VALUES(?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(session_id,block_seq) DO UPDATE SET block_id=excluded.block_id,
         skeleton_json=excluded.skeleton_json,first_row=excluded.first_row,
         row_count=excluded.row_count,block_revision=excluded.block_revision",
        params![
            session_id,
            place.block_seq,
            place.block_id,
            update.skeleton,
            place.first_row,
            place.row_count,
            place.block_revision
        ],
    )?;
    stats.blocks_written += 1;
    stats.skeleton_bytes_written += update.skeleton.len() as u64;
    Ok(())
}

/// Plan and apply one block's payload-segment writes, member by member, and
/// drop stored members the update no longer carries (a member that shrank
/// back under the strip threshold).
fn write_payload_segments(
    conn: &Connection,
    session_id: &str,
    block_seq: i64,
    payloads: &[String],
    stats: &mut SparseFlush,
) -> rusqlite::Result<()> {
    let stored = stored_payload_segments(conn, session_id, block_seq)?;
    for (member, payload) in payloads.iter().enumerate() {
        let existing = stored
            .get(member)
            .map_or(Vec::new(), |segments| segments.clone());
        let existing = existing
            .iter()
            .map(|(seg, json)| (*seg, json.as_str()))
            .collect::<Vec<_>>();
        let mut next_seg = existing.last().map_or(0, |(seg, _)| seg + 1);
        for op in anchor_ops(
            pack_payload_segments(payload, CHUNK_TARGET_BYTES),
            &existing,
        ) {
            match op {
                ChunkOp::Rewrite(seg, json) => {
                    store_payload(conn, session_id, block_seq, member as i64, seg, &json)?;
                    stats.chunks_written += 1;
                    stats.chunk_bytes_written += json.len() as u64;
                }
                ChunkOp::Append(json) => {
                    store_payload(conn, session_id, block_seq, member as i64, next_seg, &json)?;
                    next_seg += 1;
                    stats.chunks_written += 1;
                    stats.chunk_bytes_written += json.len() as u64;
                }
                ChunkOp::Truncate(keep) => {
                    conn.execute(
                        "DELETE FROM native_payloads WHERE session_id=?1 AND block_seq=?2 AND member=?3 AND seg>=?4",
                        params![session_id, block_seq, member as i64, keep],
                    )?;
                }
            }
        }
    }
    if stored.len() > payloads.len() {
        conn.execute(
            "DELETE FROM native_payloads WHERE session_id=?1 AND block_seq=?2 AND member>=?3",
            params![session_id, block_seq, payloads.len() as i64],
        )?;
    }
    Ok(())
}

/// Whether one rendered block is byte-for-byte what is already stored:
/// same skeleton, same canonical chunk packing, same payload segments.
fn block_matches_stored(
    conn: &Connection,
    session_id: &str,
    block_seq: i64,
    update: &BlockUpdate,
) -> rusqlite::Result<bool> {
    let skeleton = conn
        .query_row(
            "SELECT skeleton_json FROM native_blocks WHERE session_id=?1 AND block_seq=?2",
            params![session_id, block_seq],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(skeleton) = skeleton else {
        return Ok(false);
    };
    if update.skeleton != skeleton {
        return Ok(false);
    }
    let existing = stored_chunk_pairs(conn, session_id, block_seq)?;
    let existing = existing
        .iter()
        .map(|(ord, json)| (*ord, json.as_str()))
        .collect::<Vec<_>>();
    if anchor_ops(
        pack_stored_chunks(&update.rows, CHUNK_TARGET_BYTES),
        &existing,
    )
    .is_empty()
    {
        let stored = stored_payload_segments(conn, session_id, block_seq)?;
        if stored.len() != update.payloads.len() {
            return Ok(false);
        }
        for (member, payload) in update.payloads.iter().enumerate() {
            let existing = stored
                .get(member)
                .map_or(Vec::new(), |segments| segments.clone());
            let existing = existing
                .iter()
                .map(|(seg, json)| (*seg, json.as_str()))
                .collect::<Vec<_>>();
            if !anchor_ops(
                pack_payload_segments(payload, CHUNK_TARGET_BYTES),
                &existing,
            )
            .is_empty()
            {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    Ok(false)
}

/// One block's stored chunks as `(chunk_ord, rows_json)` in ordinal order.
pub(super) fn stored_chunk_pairs(
    conn: &Connection,
    session_id: &str,
    block_seq: i64,
) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT chunk_ord,rows_json FROM native_chunks WHERE session_id=?1 AND block_seq=?2 ORDER BY chunk_ord",
    )?;
    let stored = stmt
        .query_map(params![session_id, block_seq], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(stored)
}

/// Delete one block's rows across all three payload tables.
fn delete_block_rows(conn: &Connection, session_id: &str, block_seq: i64) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM native_payloads WHERE session_id=?1 AND block_seq=?2",
        params![session_id, block_seq],
    )?;
    conn.execute(
        "DELETE FROM native_chunks WHERE session_id=?1 AND block_seq=?2",
        params![session_id, block_seq],
    )?;
    conn.execute(
        "DELETE FROM native_blocks WHERE session_id=?1 AND block_seq=?2",
        params![session_id, block_seq],
    )?;
    Ok(())
}

/// Insert or replace one stored chunk. A rewrite targets an existing ordinal;
/// an append claims one past the last stored, which never conflicts.
fn store_chunk(
    conn: &Connection,
    session_id: &str,
    block_seq: i64,
    chunk_ord: i64,
    rows_json: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO native_chunks(session_id,block_seq,chunk_ord,rows_json,byte_len)
         VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(session_id,block_seq,chunk_ord) DO UPDATE SET rows_json=excluded.rows_json,byte_len=excluded.byte_len",
        params![session_id, block_seq, chunk_ord, rows_json, rows_json.len() as i64],
    )?;
    Ok(())
}

/// Insert or replace one stored payload segment.
fn store_payload(
    conn: &Connection,
    session_id: &str,
    block_seq: i64,
    member: i64,
    seg: i64,
    payload_json: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO native_payloads(session_id,block_seq,member,seg,payload_json,byte_len)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(session_id,block_seq,member,seg) DO UPDATE SET payload_json=excluded.payload_json,byte_len=excluded.byte_len",
        params![session_id, block_seq, member, seg, payload_json, payload_json.len() as i64],
    )?;
    Ok(())
}

/// One block's stored payload segments by member, each as `(seg, payload_json)`
/// in segment order. Blocks without stripped members read as empty.
pub(crate) fn stored_payload_segments(
    conn: &Connection,
    session_id: &str,
    block_seq: i64,
) -> rusqlite::Result<Vec<Vec<(i64, String)>>> {
    let mut stmt = conn.prepare(
        "SELECT member,seg,payload_json FROM native_payloads
         WHERE session_id=?1 AND block_seq=?2 ORDER BY member,seg",
    )?;
    let rows = stmt
        .query_map(params![session_id, block_seq], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut members: Vec<Vec<(i64, String)>> = Vec::new();
    for (member, seg, json) in rows {
        if members.len() <= member as usize {
            members.resize(member as usize + 1, Vec::new());
        }
        members[member as usize].push((seg, json));
    }
    Ok(members)
}

/// One stored block's payload: its skeleton, chunk contents in order, and the
/// full serialized values of its stripped payload members in the order their
/// placeholders appear in the skeleton.
pub(crate) fn stored_payload(
    conn: &Connection,
    session_id: &str,
    block_seq: i64,
) -> rusqlite::Result<(String, Vec<String>, Vec<String>)> {
    let skeleton = conn.query_row(
        "SELECT skeleton_json FROM native_blocks WHERE session_id=?1 AND block_seq=?2",
        params![session_id, block_seq],
        |row| row.get(0),
    )?;
    let chunks = stored_chunk_pairs(conn, session_id, block_seq)?
        .into_iter()
        .map(|(_, json)| json)
        .collect::<Vec<_>>();
    let payloads = stored_payload_segments(conn, session_id, block_seq)?
        .into_iter()
        .map(|segments| {
            segments
                .into_iter()
                .map(|(_, json)| json)
                .collect::<String>()
        })
        .collect();
    Ok((skeleton, chunks, payloads))
}

/// The block id a skeleton's raw JSON carries, if any.
fn skeleton_id(skeleton: &str) -> Option<String> {
    // A stored skeleton is not valid JSON: its rows member holds the bare
    // placeholder. Splice the placeholder out — exactly one occurrence, since
    // `block_skeleton` refuses blocks that carry the text — and read the id.
    let raw = skeleton.replacen(ROWS_PLACEHOLDER, "", 1);
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()?
        .get("id")?
        .as_str()
        .map(str::to_owned)
}
