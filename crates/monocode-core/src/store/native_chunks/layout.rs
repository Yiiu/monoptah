//! The on-disk layout of the chunked backend: its tables, and the
//! skeleton + chunks raw-JSON representation of one block.
//!
//! A block is stored as its raw JSON object — every original byte, unknown
//! fields included — extended with a trailing `rows` member holding the
//! block's projected native rows. The skeleton replaces that array's contents
//! with the literal placeholder `%NATIVE_ROWS%`; chunk rows hold the
//! comma-joined serialization of consecutive row elements, so reconstruction
//! is a pure text splice and unchanged bytes are never re-serialized.
//!
//! Two extensions keep a single block's giant payload from turning every
//! small save into a full rewrite:
//!
//! - A row element larger than [`CHUNK_TARGET_BYTES`] — a tool block projects
//!   its whole serialized `ToolCall` into one row — is split at character
//!   boundaries into pieces of at most the target. A piece that continues a
//!   split element is stored with a leading [`CHUNK_CONTINUATION`] byte
//!   (serialized JSON never contains a raw control byte, and a whole element
//!   always starts with `{`), so the splice concatenates it without the
//!   separating comma. Row-boundary chunks remain the norm.
//! - A block member whose serialized value alone exceeds the target — a long
//!   answer's `text`, a tool's `nativeInput`/`detail` — is stripped from the
//!   skeleton, so the `native_blocks` row stays small. `text` is marked with
//!   [`TEXT_SENTINEL`] and restored by joining the block's leading row group
//!   (the projection splits it at row boundaries, so the join is exact — and
//!   the writer verifies the join before stripping). Tool payload members are
//!   marked with [`PAYLOAD_PLACEHOLDER`] and restored from segmented copies in
//!   `native_payloads`, substituted in the order their placeholders appear.
use crate::transcript::Row;
use rusqlite::Connection;

/// The envelope member carrying a block's projected native rows.
pub(super) const ROWS_MEMBER: &str = "rows";
/// The literal that replaces the rows-array contents in a stored skeleton.
pub(super) const ROWS_PLACEHOLDER: &str = "%NATIVE_ROWS%";
/// The value marking a stripped `text` member, restored from the rows.
pub(super) const TEXT_SENTINEL: &str = "%NATIVE_ROWS_TEXT%";
/// The value marking a stripped tool payload member, restored from
/// `native_payloads`.
pub(super) const PAYLOAD_PLACEHOLDER: &str = "%NATIVE_PAYLOAD%";
/// The byte prefix marking a chunk that continues a byte-split row element.
pub(super) const CHUNK_CONTINUATION: char = '\u{1}';

/// The chunk width a block's rows and payload members are packed into. The
/// packing is canonical — the same content always produces the same chunks —
/// so a save rewrites only the chunks the changed bytes actually moved, never
/// the whole payload. Also the size over which a block member is stripped
/// from its skeleton.
pub(crate) const CHUNK_TARGET_BYTES: usize = 64 * 1024;

/// The tool members that can hold an unbounded payload. `tool()` prefers
/// `nativeInput` and falls back through the others, so any of them can be the
/// giant copy.
const TOOL_PAYLOAD_MEMBERS: [&str; 4] = ["nativeInput", "rawInput", "raw_input", "input"];

const NATIVE_CHUNKS_TABLES: &str = "
CREATE TABLE IF NOT EXISTS native_sessions(
  session_id TEXT PRIMARY KEY,
  mode_version INTEGER NOT NULL,
  session_revision INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS native_blocks(
  session_id TEXT,
  block_seq INTEGER,
  block_id TEXT,
  skeleton_json TEXT NOT NULL,
  first_row INTEGER NOT NULL,
  row_count INTEGER NOT NULL,
  block_revision INTEGER NOT NULL,
  PRIMARY KEY(session_id, block_seq)
);
CREATE TABLE IF NOT EXISTS native_chunks(
  session_id TEXT,
  block_seq INTEGER,
  chunk_ord INTEGER,
  rows_json TEXT NOT NULL,
  byte_len INTEGER NOT NULL,
  PRIMARY KEY(session_id, block_seq, chunk_ord)
);
CREATE TABLE IF NOT EXISTS native_payloads(
  session_id TEXT,
  block_seq INTEGER,
  member INTEGER NOT NULL,
  seg INTEGER NOT NULL,
  payload_json TEXT NOT NULL,
  byte_len INTEGER NOT NULL,
  PRIMARY KEY(session_id, block_seq, member, seg)
);
";

/// Create the chunked-backend tables. New databases get them in the same
/// transaction as the desktop schema; conversion creates them for a session
/// it switches over. The statements are idempotent.
pub(crate) fn ensure_native_chunks_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(NATIVE_CHUNKS_TABLES)
}

/// Refuse a source document that already carries one of the splice markers:
/// a stored skeleton containing the literal text could never be spliced back
/// unambiguously.
fn refuse_reserved_markers(raw: &str) -> Result<(), String> {
    for reserved in [ROWS_PLACEHOLDER, TEXT_SENTINEL, PAYLOAD_PLACEHOLDER] {
        if raw.contains(reserved) {
            return Err(format!(
                "Block JSON contains the reserved marker {reserved}, so its skeleton could not \
                 be spliced unambiguously"
            ));
        }
    }
    Ok(())
}

/// Build one block's stored form from its raw JSON and projected rows. The
/// skeleton keeps every original byte (unknown fields verbatim) and gains a
/// trailing `rows` member whose array contents are the placeholder; the
/// returned elements are the serialized rows chunks are cut from.
pub(crate) fn block_skeleton(raw: &str, rows: &[Row]) -> Result<(String, Vec<String>), String> {
    refuse_reserved_markers(raw)?;
    splice_skeleton(raw, rows)
}

/// The skeleton splice, without the reserved-marker guard — for a raw that
/// was already checked (the stripped form [`store_block_form`] builds
/// legitimately carries the strip sentinels).
fn splice_skeleton(raw: &str, rows: &[Row]) -> Result<(String, Vec<String>), String> {
    let raw = raw.trim();
    let block: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| format!("Stored block is not valid JSON: {error}"))?;
    if !block.is_object() {
        return Err("A stored block must be a JSON object".into());
    }
    if block[ROWS_MEMBER] != serde_json::Value::Null {
        return Err(format!(
            "A block cannot carry its own \"{ROWS_MEMBER}\" member; the chunked backend reserves it"
        ));
    }
    let elements = rows
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let body = raw
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .map(str::trim)
        .ok_or("A stored block must be a serialized JSON object")?;
    let skeleton = if body.is_empty() {
        format!("{{\"{ROWS_MEMBER}\":[{ROWS_PLACEHOLDER}]}}")
    } else {
        format!("{{{body},\"{ROWS_MEMBER}\":[{ROWS_PLACEHOLDER}]}}")
    };
    Ok((skeleton, elements))
}

/// One block's stored form: the skeleton (giant members stripped) and the
/// serialized row elements, plus the serialized values of the members moved
/// into `native_payloads`, in the order their placeholders appear.
pub(crate) struct StoredForm {
    pub skeleton: String,
    pub elements: Vec<String>,
    pub payloads: Vec<String>,
}

/// Build one block's full stored form, stripping members whose serialized
/// value alone exceeds [`CHUNK_TARGET_BYTES`] so the `native_blocks` row
/// stays bounded. A block without giant members stores exactly the bytes
/// [`block_skeleton`] produces today.
pub(crate) fn store_block_form(
    value: &serde_json::Value,
    raw: &str,
    rows: &[Row],
) -> Result<StoredForm, String> {
    // The marker guard runs against the source document, before any member is
    // stripped: the stripped form below legitimately carries the strip
    // sentinels in place of the members it moved into `native_payloads`.
    refuse_reserved_markers(raw)?;
    if let Some((stripped_raw, payloads)) = strip_giant_members(value, rows, raw)? {
        let (skeleton, elements) = splice_skeleton(&stripped_raw, rows)?;
        return Ok(StoredForm {
            skeleton,
            elements,
            payloads,
        });
    }
    let (skeleton, elements) = block_skeleton(raw, rows)?;
    Ok(StoredForm {
        skeleton,
        elements,
        payloads: Vec::new(),
    })
}

/// Members worth stripping, with why `text` is safe to restore from rows.
fn strip_giant_members(
    value: &serde_json::Value,
    rows: &[Row],
    raw: &str,
) -> Result<Option<(String, Vec<String>)>, String> {
    // `text` is only stripped when joining the block's leading first-flag row
    // group reproduces it byte for byte: the projection splits it at row
    // boundaries, so the join is the exact inverse. Any block whose rows are
    // not a plain split of its text (an interjection's serialized note, a
    // hidden draft with no rows) keeps its text inline.
    let strip_text = value["text"].as_str().is_some_and(|text| {
        text.len() > CHUNK_TARGET_BYTES && !rows.is_empty() && {
            let end = rows
                .iter()
                .skip(1)
                .position(|row| row.first)
                .map_or(rows.len(), |position| position + 1);
            let joined: String = rows[..end].iter().map(|row| row.text.as_str()).collect();
            joined == text
        }
    });
    let mut tool_members: Vec<&str> = Vec::new();
    for (key, member) in value["tool"].as_object().into_iter().flatten() {
        if TOOL_PAYLOAD_MEMBERS.contains(&key.as_str())
            && member
                .as_str()
                .is_some_and(|text| text.len() > CHUNK_TARGET_BYTES)
        {
            tool_members.push(key.as_str());
        }
    }
    if !strip_text && tool_members.is_empty() {
        return Ok(None);
    }
    let mut stripped = value.clone();
    if strip_text {
        stripped["text"] = serde_json::Value::String(TEXT_SENTINEL.into());
    }
    let mut payloads = Vec::new();
    for key in tool_members {
        payloads.push(
            serde_json::to_string(&stripped["tool"][key]).map_err(|error| error.to_string())?,
        );
        stripped["tool"][key] = serde_json::Value::String(PAYLOAD_PLACEHOLDER.into());
    }
    let stripped_raw = serde_json::to_string(&stripped).map_err(|error| error.to_string())?;
    debug_assert!(!stripped_raw.contains(raw));
    Ok(Some((stripped_raw, payloads)))
}

/// The rows-array content the chunks hold: row-boundary chunks joined with a
/// separating comma, byte-split pieces concatenated without one.
fn rows_content(chunks: &[String]) -> Result<String, String> {
    let mut content = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        if index == 0 {
            if chunk.starts_with(CHUNK_CONTINUATION) {
                return Err("A stored chunk cannot begin with the continuation marker".into());
            }
            content.push_str(chunk);
        } else if let Some(rest) = chunk.strip_prefix(CHUNK_CONTINUATION) {
            content.push_str(rest);
        } else {
            content.push(',');
            content.push_str(chunk);
        }
    }
    Ok(content)
}

/// Reconstruct a block's stored raw JSON: a pure text splice of the chunks in
/// place of the placeholder, then of each payload member's segments in place
/// of its placeholder. Unchanged skeleton and chunk bytes come back verbatim,
/// so untouched parts of a block never get re-serialized.
pub(crate) fn block_raw_json(
    skeleton: &str,
    chunks: &[String],
    payloads: &[String],
) -> Result<String, String> {
    match skeleton.matches(ROWS_PLACEHOLDER).count() {
        1 => {}
        0 => return Err("A stored skeleton has no rows placeholder".into()),
        _ => return Err("A stored skeleton has more than one rows placeholder".into()),
    }
    let quoted_placeholder = format!("\"{PAYLOAD_PLACEHOLDER}\"");
    let slots = skeleton.matches(&quoted_placeholder).count();
    if slots != payloads.len() {
        return Err(format!(
            "A stored skeleton has {slots} payload placeholders but {} payload members were read",
            payloads.len()
        ));
    }
    // Payload placeholders are substituted while the skeleton still holds the
    // rows placeholder, so a payload value can never be mistaken for rows
    // content (the reserved-marker guards refuse that overlap at build).
    let mut raw = skeleton.to_owned();
    for payload in payloads {
        raw = raw.replacen(&quoted_placeholder, payload, 1);
    }
    let content = rows_content(chunks)?;
    let raw = raw.replacen(ROWS_PLACEHOLDER, &content, 1);
    serde_json::from_str::<serde::de::IgnoredAny>(&raw)
        .map_err(|error| format!("Stored chunks do not splice into valid JSON: {error}"))?;
    Ok(raw)
}

/// The stored document minus its envelope member: the desktop block value a
/// caller merges into and re-renders. [`block_raw_json`] keeps the `rows`
/// envelope because splicing needs it; this drops it again and restores a
/// stripped `text` from the rows, so the value can round-trip back through
/// [`store_block_form`].
pub(crate) fn block_value(
    skeleton: &str,
    chunks: &[String],
    payloads: &[String],
) -> Result<serde_json::Value, String> {
    let raw = block_raw_json(skeleton, chunks, payloads)?;
    let mut document: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|error| format!("Stored block is not valid JSON: {error}"))?;
    let rows = document[ROWS_MEMBER].clone();
    let Some(object) = document.as_object_mut() else {
        return Err("A stored block must be a JSON object".into());
    };
    object
        .remove(ROWS_MEMBER)
        .ok_or("A stored skeleton has no rows member")?;
    if document["text"].as_str() == Some(TEXT_SENTINEL) {
        let rows: Vec<Row> = serde_json::from_value(rows)
            .map_err(|error| format!("Stored rows do not parse: {error}"))?;
        let end = rows
            .iter()
            .skip(1)
            .position(|row| row.first)
            .map_or(rows.len(), |position| position + 1);
        let joined: String = rows[..end].iter().map(|row| row.text.as_str()).collect();
        document["text"] = serde_json::Value::String(joined);
    }
    Ok(document)
}

/// The rows a block's chunks hold, parsed without touching its skeleton.
pub(crate) fn parse_rows(chunks: &[String]) -> Result<Vec<Row>, String> {
    let content = rows_content(chunks)?;
    let rows: Vec<Row> = serde_json::from_str(&format!("[{content}]"))
        .map_err(|error| format!("Stored rows do not parse: {error}"))?;
    Ok(rows)
}

/// Pack serialized row elements into chunk contents of at most `target_bytes`
/// (separating commas included). Row boundaries are never split: a row larger
/// than the target becomes a chunk of its own. No rows means no chunks.
/// Reference packing without oversized-row splitting; the stored packing
/// reduces to it when no element exceeds the target, which the tests pin.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn pack_row_chunks(elements: &[String], target_bytes: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for element in elements {
        if !current.is_empty() && current.len() + 1 + element.len() > target_bytes {
            chunks.push(std::mem::take(&mut current));
        }
        if current.is_empty() {
            current.push_str(element);
        } else {
            current.push(',');
            current.push_str(element);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Cut one oversized text into pieces of at most `target_bytes` at character
/// boundaries, so the pieces stay valid UTF-8 inside a TEXT column.
fn split_oversized(text: &str, target_bytes: usize, mark_continuations: bool) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut start = 0;
    while text.len() - start > target_bytes {
        let mut cut = start + target_bytes;
        while cut > start && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        let piece = &text[start..cut];
        pieces.push(piece_and_marker(piece, start == 0, mark_continuations));
        start = cut;
    }
    pieces.push(piece_and_marker(
        &text[start..],
        start == 0,
        mark_continuations,
    ));
    pieces
}

fn piece_and_marker(piece: &str, is_first: bool, mark_continuations: bool) -> String {
    if is_first || !mark_continuations {
        piece.to_owned()
    } else {
        format!("{CHUNK_CONTINUATION}{piece}")
    }
}

/// The canonical packing of a block's stored row chunks: the same greedy
/// row-boundary packing as [`pack_row_chunks`], except that a row element
/// larger than the target — which would be one giant chunk — is split at
/// character boundaries into pieces of at most the target. Continuation
/// pieces carry [`CHUNK_CONTINUATION`] so the splice concatenates them
/// without a separating comma; the stored layout is always this packing, so
/// unchanged pieces are anchored and skipped on the next save.
pub(crate) fn pack_stored_chunks(elements: &[String], target_bytes: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for element in elements {
        if element.len() > target_bytes {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            chunks.extend(split_oversized(element, target_bytes, true));
            continue;
        }
        if !current.is_empty() && current.len() + 1 + element.len() > target_bytes {
            chunks.push(std::mem::take(&mut current));
        }
        if current.is_empty() {
            current.push_str(element);
        } else {
            current.push(',');
            current.push_str(element);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// The canonical segments of one stripped member's serialized value: pieces of
/// at most `target_bytes` at character boundaries, no markers (the segments
/// live in their own table with explicit positions).
pub(crate) fn pack_payload_segments(value: &str, target_bytes: usize) -> Vec<String> {
    split_oversized(value, target_bytes, false)
}
