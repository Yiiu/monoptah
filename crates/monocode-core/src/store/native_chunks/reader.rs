//! The chunked backend's read side: full-history block iteration and windowed
//! row pagination, both driven by the stored block index so absolute row
//! numbers are whatever `native_blocks.first_row` assigned at write time — a
//! reader never renumbers, and pagination never shifts the row origin.
use super::layout::{block_value, parse_rows};
use super::writer::{block_index, stored_chunk_pairs, stored_payload};
use rusqlite::Connection;
use std::collections::VecDeque;

/// A reconstructed stored block: its raw JSON document minus the chunked
/// backend's envelope, with stripped members restored.
pub(crate) type RawBlock = serde_json::Value;

/// A session's stored blocks, reconstructed in order. Chunks and payload
/// segments are read one block at a time, lazily, so a caller that stops early
/// (search walking to a match, the today backfill reading the first user
/// message) never pays for the whole history. A failing block-index read
/// surfaces as the one and only item, an `Err`.
pub(crate) fn iter_full_history<'a>(
    conn: &'a Connection,
    session_id: &'a str,
) -> impl Iterator<Item = Result<RawBlock, String>> + 'a {
    let entries = match block_index(conn, session_id) {
        Ok(blocks) => blocks.into_iter().map(Ok).collect::<VecDeque<_>>(),
        Err(error) => VecDeque::from([Err(error.to_string())]),
    };
    entries
        .into_iter()
        .map(move |entry| -> Result<RawBlock, String> {
            let stored = entry?;
            let (skeleton, chunks, payloads) = stored_payload(conn, session_id, stored.block_seq)
                .map_err(|error| error.to_string())?;
            block_value(&skeleton, &chunks, &payloads)
        })
}

/// Up to `limit` rows starting at absolute row `first_row`, in order. Absolute
/// numbers come from the stored extents, so paging with a different limit or
/// offset returns the same rows under the same numbers. Only the blocks
/// covering the window are read. A session with no chunked marker reads as
/// empty (the store layer dispatches blob sessions elsewhere).
pub(crate) fn page_rows(
    conn: &Connection,
    session_id: &str,
    first_row: i64,
    limit: usize,
) -> Result<Vec<Row>, String> {
    let index = block_index(conn, session_id).map_err(|error| error.to_string())?;
    let limit = limit.min(1 << 40);
    let mut rows = Vec::new();
    if limit == 0 {
        return Ok(rows);
    }
    for stored in index {
        if stored.row_count == 0 || stored.first_row + stored.row_count <= first_row {
            continue;
        }
        let chunks = stored_chunk_pairs(conn, session_id, stored.block_seq)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|(_, json)| json)
            .collect::<Vec<_>>();
        let mut block_rows = parse_rows(&chunks)?;
        // Older writers projected goal blocks as system rows. Check the
        // original block role so ordinary system text cannot become goal state.
        if block_rows.first().is_some_and(|row| {
            row.role == "system" && row.text.starts_with('{') && row.text.contains("\"phase\":")
        }) {
            let (skeleton, chunks, payloads) = stored_payload(conn, session_id, stored.block_seq)
                .map_err(|error| error.to_string())?;
            if block_value(&skeleton, &chunks, &payloads)?["role"] == "goal" {
                for row in block_rows.iter_mut().take_while(|row| row.role == "system") {
                    row.role = "goal".into();
                }
            }
        }
        if block_rows.len() as i64 != stored.row_count {
            return Err(format!(
                "Block {} of session {session_id} stores {} rows but holds {}",
                stored.block_seq,
                stored.row_count,
                block_rows.len()
            ));
        }
        for (offset, row) in block_rows.into_iter().enumerate() {
            if stored.first_row + (offset as i64) < first_row {
                continue;
            }
            rows.push(row);
            if rows.len() >= limit {
                return Ok(rows);
            }
        }
    }
    Ok(rows)
}

use crate::transcript::Row;
