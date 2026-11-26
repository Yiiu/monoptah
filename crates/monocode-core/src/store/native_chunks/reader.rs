//! The chunked backend's read side: full-history block iteration and windowed
//! row pagination, both driven by the stored block index so absolute row
//! numbers are whatever `native_blocks.first_row` assigned at write time — a
//! reader never renumbers, and pagination never shifts the row origin.
use super::layout::{block_value, parse_rows};
use super::writer::{block_index, stored_chunk_pairs, stored_payload};
use crate::transcript::Row;
use rusqlite::{params, Connection, OptionalExtension};
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
            read_block(conn, session_id, entry?.block_seq)
        })
}

fn read_block(conn: &Connection, session_id: &str, seq: i64) -> Result<RawBlock, String> {
    let (skeleton, chunks, payloads) =
        stored_payload(conn, session_id, seq).map_err(|error| error.to_string())?;
    block_value(&skeleton, &chunks, &payloads)
}

enum FirstRole {
    Prompt,
    MaybeUser,
    Other,
    Unknown,
}

impl FirstRole {
    fn from_prefix(prefix: Option<&str>) -> Self {
        // Both native layouts serialize typed Rows with role first. An older
        // encoding we don't recognize is inspected through the ordinary decoder.
        let Some(role) = prefix
            .and_then(|prefix| prefix.strip_prefix("{\"role\":\""))
            .and_then(|rest| rest.split_once("\",\"text\":"))
            .map(|(role, _)| role)
        else {
            return Self::Unknown;
        };
        match role {
            "user" | "mono-system" => Self::Prompt,
            // A user block can project to an interjection or metadata-only
            // card and still carry a raw operator marker.
            "interjection"
            | "mono-document"
            | "mono-note"
            | "mono-launch"
            | "mono-card"
            | crate::transcript::attachments::ROLE => Self::MaybeUser,
            role if !role.is_empty()
                && role
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'-') =>
            {
                Self::Other
            }
            _ => Self::Unknown,
        }
    }
}

struct RowBlock {
    seq: i64,
    first: i64,
    count: i64,
    role: FirstRole,
}

impl RowBlock {
    fn end(&self) -> usize {
        (self.first + self.count) as usize
    }

    fn extent(self) -> (i64, i64, i64) {
        (self.seq, self.first, self.count)
    }

    fn prompt(&self, conn: &Connection, session_id: &str) -> Result<Option<usize>, String> {
        let offset = match self.role {
            FirstRole::Prompt => Some(0),
            FirstRole::Unknown => {
                let chunks = stored_chunk_pairs(conn, session_id, self.seq)
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .map(|(_, chunk)| chunk)
                    .collect::<Vec<_>>();
                parse_rows(&chunks)?
                    .iter()
                    .position(|row| row.first && row.is_prompt())
            }
            _ => None,
        };
        Ok(offset.map(|offset| self.first as usize + offset))
    }
}

fn walk_roles(
    conn: &Connection,
    session_id: &str,
    from_seq: i64,
    newest_first: bool,
    mut visit: impl FnMut(RowBlock) -> Result<bool, String>,
) -> Result<(), String> {
    let (bound, order) = if newest_first {
        ("<=", "DESC")
    } else {
        (">=", "ASC")
    };
    let mut statement = conn
        .prepare(&format!(
            "SELECT b.block_seq, b.first_row, b.row_count, substr(c.rows_json, 1, 32)
             FROM native_blocks b
             LEFT JOIN native_chunks c ON c.session_id=b.session_id
                 AND c.block_seq=b.block_seq AND c.chunk_ord=0
             WHERE b.session_id=?1 AND b.block_seq {bound} ?2 AND b.row_count>0
             ORDER BY b.block_seq {order}"
        ))
        .map_err(|error| error.to_string())?;
    let records = statement
        .query_map(params![session_id, from_seq], |row| {
            let prefix: Option<String> = row.get(3)?;
            Ok(RowBlock {
                seq: row.get(0)?,
                first: row.get(1)?,
                count: row.get(2)?,
                role: FirstRole::from_prefix(prefix.as_deref()),
            })
        })
        .map_err(|error| error.to_string())?;
    for entry in records {
        if !visit(entry.map_err(|error| error.to_string())?)? {
            break;
        }
    }
    Ok(())
}

/// Only blocks that might contribute a prompt receipt or operator marker,
/// paired with their original absolute first row.
pub(crate) fn prompt_sources<'a>(
    conn: &'a Connection,
    session_id: &'a str,
) -> Result<impl Iterator<Item = Result<(usize, RawBlock), String>> + 'a, String> {
    let mut prompts = Vec::new();
    walk_roles(conn, session_id, i64::MIN, false, |block| {
        if !matches!(block.role, FirstRole::Other) {
            prompts.push(block);
        }
        Ok(true)
    })?;
    Ok(prompts.into_iter().map(move |block| {
        Ok((
            block.first as usize,
            read_block(conn, session_id, block.seq)?,
        ))
    }))
}

// Row extents increase in block sequence order. Seek the predecessor of
// each midpoint through the primary key, including when block sequences
// have gaps. Only metadata for O(log(sequence range)) blocks is fetched.
fn anchor_block(conn: &Connection, session_id: &str, anchor: usize) -> Result<Option<i64>, String> {
    let Ok(anchor) = i64::try_from(anchor) else {
        return Ok(None);
    };
    let (min, max): (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT
                (SELECT min(block_seq) FROM native_blocks WHERE session_id=?1),
                (SELECT max(block_seq) FROM native_blocks WHERE session_id=?1)",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    let (Some(mut low), Some(mut high)) = (min, max) else {
        return Ok(None);
    };
    let mut predecessor = conn
        .prepare(
            "SELECT block_seq, first_row, row_count FROM native_blocks
             WHERE session_id=?1 AND block_seq<=?2 AND row_count>0
             ORDER BY block_seq DESC LIMIT 1",
        )
        .map_err(|error| error.to_string())?;
    while low <= high {
        let midpoint = low.midpoint(high);
        let block: Option<(i64, i64, i64)> = predecessor
            .query_row(params![session_id, midpoint], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()
            .map_err(|error| error.to_string())?;
        match block {
            Some((seq, first, _)) if anchor < first => {
                let Some(before) = seq.checked_sub(1) else {
                    break;
                };
                high = before;
            }
            Some((seq, first, count)) if anchor - first < count => return Ok(Some(seq)),
            _ => {
                let Some(after) = midpoint.checked_add(1) else {
                    break;
                };
                low = after;
            }
        }
    }
    Ok(None)
}

#[derive(Default)]
struct TurnScan {
    blocks: Vec<RowBlock>,
    prompts: Vec<usize>,
}

fn scan_turns(
    conn: &Connection,
    session_id: &str,
    from_seq: i64,
    newest_first: bool,
    anchor: Option<usize>,
    turns: usize,
) -> Result<TurnScan, String> {
    let mut scan = TurnScan::default();
    walk_roles(conn, session_id, from_seq, newest_first, |block| {
        if let Some(prompt) = block.prompt(conn, session_id)? {
            if anchor.is_none_or(|anchor| (prompt <= anchor) == newest_first) {
                scan.prompts.push(prompt);
            }
        }
        scan.blocks.push(block);
        Ok(scan.prompts.len() < turns)
    })?;
    Ok(scan)
}

/// Display-only preview. The caller holds a read transaction, so the prompt
/// boundaries and the rows selected by them come from one snapshot.
pub(crate) fn history_window(
    conn: &Connection,
    session_id: &str,
    anchor: Option<usize>,
    turns: usize,
) -> Result<Option<(usize, Vec<Row>)>, String> {
    let turns = turns.max(1);
    let Some(anchor) = anchor else {
        let tail = scan_turns(conn, session_id, i64::MAX, true, None, turns)?;
        let Some(last) = tail.blocks.first() else {
            return Ok(Some((0, Vec::new())));
        };
        if tail.prompts.is_empty() {
            return Ok(None);
        }
        let first = if tail.prompts.len() == turns {
            tail.prompts[turns - 1]
        } else {
            0
        };
        return read_window(
            conn,
            session_id,
            first,
            last.end(),
            tail.blocks.into_iter().rev(),
        );
    };

    let Some(seq) = anchor_block(conn, session_id, anchor)? else {
        return Ok(None);
    };
    let mut older = scan_turns(conn, session_id, seq, true, Some(anchor), turns / 2 + 1)?;
    let room = turns - older.prompts.len();
    let newer = scan_turns(
        conn,
        session_id,
        seq,
        false,
        Some(anchor),
        room.saturating_add(1),
    )?;
    if older.prompts.is_empty() && newer.prompts.is_empty() {
        return Ok(None);
    }
    // A short tail leaves room for earlier turns than initially requested.
    let extra = room.saturating_sub(newer.prompts.len());
    if extra > 0 {
        if let Some(before) = older
            .blocks
            .last()
            .and_then(|block| block.seq.checked_sub(1))
        {
            let more = scan_turns(conn, session_id, before, true, Some(anchor), extra)?;
            older.prompts.extend(more.prompts);
            older.blocks.extend(more.blocks);
        }
    }
    let first = if older.blocks.last().is_some_and(|block| block.first == 0) {
        0
    } else {
        older.prompts.last().copied().unwrap_or(0)
    };
    let end = newer
        .prompts
        .get(room)
        .copied()
        .or_else(|| newer.blocks.last().map(RowBlock::end))
        .ok_or("The history anchor has missing rows")?;
    let selected = older
        .blocks
        .into_iter()
        .rev()
        .chain(newer.blocks.into_iter().skip(1));
    read_window(conn, session_id, first, end, selected)
}

fn read_window(
    conn: &Connection,
    session_id: &str,
    first: usize,
    end: usize,
    blocks: impl Iterator<Item = RowBlock>,
) -> Result<Option<(usize, Vec<Row>)>, String> {
    let count = end
        .checked_sub(first)
        .ok_or("The history window has invalid row boundaries")?;
    let rows = rows_for_blocks(
        conn,
        session_id,
        first as i64,
        count,
        blocks.map(RowBlock::extent),
    )?;
    if rows.len() != count {
        return Err("The history window has missing rows".into());
    }
    Ok(Some((first, rows)))
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
    let blocks = block_index(conn, session_id)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|block| (block.block_seq, block.first_row, block.row_count));
    rows_for_blocks(conn, session_id, first_row, limit, blocks)
}

fn rows_for_blocks(
    conn: &Connection,
    session_id: &str,
    first_row: i64,
    limit: usize,
    blocks: impl IntoIterator<Item = (i64, i64, i64)>,
) -> Result<Vec<Row>, String> {
    let limit = limit.min(1 << 40);
    let mut rows = Vec::new();
    if limit == 0 {
        return Ok(rows);
    }
    for (seq, first, count) in blocks {
        if count == 0 || first + count <= first_row {
            continue;
        }
        let chunks = stored_chunk_pairs(conn, session_id, seq)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|(_, json)| json)
            .collect::<Vec<_>>();
        let mut block_rows = parse_rows(&chunks)?;
        // Older writers projected goal blocks as system rows. Check the
        // original block role so ordinary system text cannot become goal state.
        if block_rows.first().is_some_and(|row| row.role == "system") {
            // Inspect the small skeleton, without rebuilding large system blocks.
            // A full-block envelope stores the original role in its payload.
            let goal: Option<bool> = conn
                .query_row(
                    "SELECT CASE WHEN substr(skeleton_json,1,1) <> '[' THEN
                    coalesce(json_extract(replace(skeleton_json,?3,''),'$.role')='goal',0)
                 END FROM native_blocks WHERE session_id=?1 AND block_seq=?2",
                    rusqlite::params![session_id, seq, super::layout::ROWS_PLACEHOLDER],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            let goal = match goal {
                Some(goal) => goal,
                None => {
                    let (skeleton, chunks, payloads) =
                        stored_payload(conn, session_id, seq).map_err(|error| error.to_string())?;
                    block_value(&skeleton, &chunks, &payloads)?["role"] == "goal"
                }
            };
            if goal {
                for row in block_rows.iter_mut().take_while(|row| row.role == "system") {
                    row.role = "goal".into();
                }
            }
        }
        if block_rows.len() as i64 != count {
            return Err(format!(
                "Block {} of session {session_id} stores {} rows but holds {}",
                seq,
                count,
                block_rows.len()
            ));
        }
        for (offset, row) in block_rows.into_iter().enumerate() {
            if first + (offset as i64) < first_row {
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
