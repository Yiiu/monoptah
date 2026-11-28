//! Recovery for a final local reply that the transcript store could not commit.
use super::{Database, Row, Store, locked};
use monocode_core::files::remote_write::sha256_hex;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Owner {
    id: String,
    provider: String,
    provider_id: Option<String>,
    cwd: String,
}

fn owner(db: &Store, id: i64) -> Result<Option<Owner>, String> {
    let Some(summary) = db.sessions()?.into_iter().find(|summary| summary.id == id) else {
        return Ok(None);
    };
    if db.require_local_run(id).is_err() {
        return Ok(None);
    }
    Ok(Some(Owner {
        id: db.persisted_session_id(id)?,
        provider: summary.provider,
        provider_id: summary.provider_id,
        cwd: summary.cwd,
    }))
}

#[derive(Serialize, Deserialize)]
struct Reply {
    session: i64,
    owner: Owner,
    before: String,
    after: String,
    ranges: Vec<(usize, Vec<Row>)>,
}

struct Pending {
    reply: Reply,
    journal: Option<PathBuf>,
    journaled: bool,
}

type Key = (usize, i64);
type Entry = Arc<Mutex<Pending>>;
static PENDING: LazyLock<Mutex<HashMap<Key, Entry>>> = LazyLock::new(Default::default);
pub(crate) const REPLY_RETAINED: &str = "The reply is kept and will be saved again.";
pub(crate) const REPLY_IN_MEMORY: &str =
    "The reply is kept in memory and will be saved again. Keep the app open until it saves.";

pub(crate) fn is_retained_status(status: &str) -> bool {
    status.contains(REPLY_RETAINED) || status.contains(REPLY_IN_MEMORY)
}

fn key(store: &Database, session: i64) -> Key {
    (Arc::as_ptr(store) as usize, session)
}

fn pending() -> std::sync::MutexGuard<'static, HashMap<Key, Entry>> {
    PENDING.lock().unwrap_or_else(|error| error.into_inner())
}

fn journal_dir(db: &Store) -> Option<PathBuf> {
    db.data_dir()?;
    let mut dir = db.database_path().as_os_str().to_owned();
    dir.push(".gpui-unsaved");
    Some(dir.into())
}

fn row_content(row: &Row) -> (&str, &str, bool) {
    (&row.role, &row.text, row.first)
}

fn revision(rows: &[Row]) -> Result<String, String> {
    // A later turn receipt may enrich sent_at through Store::load; the row's
    // text, role and boundary are the parts this recovery would modify.
    let rows: Vec<_> = rows.iter().map(row_content).collect();
    serde_json::to_vec(&rows)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|error| error.to_string())
}

fn apply(rows: &mut Vec<Row>, ranges: &[(usize, Vec<Row>)]) -> bool {
    for (start, replacement) in ranges {
        if *start > rows.len() {
            return false;
        }
        let end = start.saturating_add(replacement.len()).min(rows.len());
        rows.splice(*start..end, replacement.iter().cloned());
    }
    true
}

/// Keep the exact pending row ranges. Confirm that the saved user turns and
/// unaffected rows still belong to this run before taking a recovery baseline.
pub(super) fn retain(
    store: &Database,
    session: i64,
    expected: monocode_core::store::ReplyRecoveryRevision,
    finished_rows: &[Row],
    ranges: &[(usize, &[Row])],
) -> Result<Option<bool>, String> {
    let (reply, journal) = {
        let db = locked(store)?;
        let Some(owner) = owner(&db, session)? else {
            return Ok(None);
        };
        let Some(baseline) = db.reply_recovery_baseline(session, expected)? else {
            return Ok(None);
        };
        let mut saved = baseline.rows;
        if saved.len() > finished_rows.len()
            || finished_rows
                .iter()
                .skip(saved.len())
                .any(|row| row.role == "user")
            || saved.iter().zip(finished_rows).any(|(saved, next)| {
                (saved.role == "user" || next.role == "user")
                    && row_content(saved) != row_content(next)
            })
        {
            return Ok(None);
        }
        let before = revision(&saved)?;
        let ranges: Vec<_> = ranges
            .iter()
            .map(|(start, rows)| (*start, rows.to_vec()))
            .collect();
        if !apply(&mut saved, &ranges)
            || saved
                .iter()
                .map(row_content)
                .ne(finished_rows.iter().map(row_content))
        {
            return Ok(None);
        }
        let after = revision(&saved)?;
        if before == after {
            return Ok(None);
        }
        let journal = journal_dir(&db)
            .map(|dir| dir.join(format!("{session}_{}.json", monocode_core::mono::new_id())));
        (
            Reply {
                session,
                owner,
                before,
                after,
                ranges,
            },
            journal,
        )
    };
    let mut entry = Pending {
        reply,
        journal,
        journaled: false,
    };
    entry.write_journal();
    let journaled = entry.journaled;
    let entry = Arc::new(Mutex::new(entry));
    pending().insert(key(store, session), entry.clone());
    retry_later(store.clone(), session, entry);
    Ok(Some(journaled))
}

impl Pending {
    fn write_journal(&mut self) {
        if self.journaled {
            return;
        }
        if let Some(path) = &self.journal {
            self.journaled = serde_json::to_vec(&self.reply)
                .is_ok_and(|json| write_private(path, &json).is_ok());
        }
    }
}

fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = path.parent().expect("journal has a directory");
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    let partial = path.with_extension(format!("{}.partial", monocode_core::mono::new_id()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&partial)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&partial, path)?;
        #[cfg(unix)]
        {
            fs::File::open(dir)?.sync_all()?;
            if let Some(parent) = dir.parent() {
                fs::File::open(parent)?.sync_all()?;
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(partial);
    }
    result
}

/// Replay is conditional on the whole durable transcript, not just the first
/// row of the final turn. A newer history keeps its rows and the journal is
/// set aside. A replay interrupted after commit is safe to resume.
pub(super) fn save_pending(store: &Database, session: i64) -> Result<(), String> {
    let Some(entry) = pending().get(&key(store, session)).cloned() else {
        return Ok(());
    };
    save_entry(store, session, &entry)
}

fn save_entry(store: &Database, session: i64, entry: &Entry) -> Result<(), String> {
    let mut reply = entry.lock().unwrap_or_else(|error| error.into_inner());
    if !pending()
        .get(&key(store, session))
        .is_some_and(|current| Arc::ptr_eq(current, entry))
    {
        return Ok(());
    }
    let valid = (|| {
        let mut db = locked(store)?;
        if owner(&db, session)?.as_ref() != Some(&reply.reply.owner) {
            return Ok(false);
        }
        let mut saved = db.load(session)?.rows;
        let before = revision(&saved)?;
        if before == reply.reply.after {
            return Ok(true);
        }
        if before != reply.reply.before
            || !apply(&mut saved, &reply.reply.ranges)
            || revision(&saved)? != reply.reply.after
        {
            return Ok(false);
        }
        let ranges: Vec<_> = reply
            .reply
            .ranges
            .iter()
            .map(|(start, rows)| (*start, rows.as_slice()))
            .collect();
        db.write_row_ranges(session, &ranges)?;
        Ok::<_, String>(true)
    })();
    let valid = match valid {
        Ok(valid) => valid,
        Err(error) => {
            // Space can free for the journal before SQLite can commit.
            reply.write_journal();
            return Err(error);
        }
    };
    if let Some(path) = &reply.journal {
        let result = if valid {
            fs::remove_file(path)
        } else {
            let stale =
                path.with_extension(format!("stale-{}.json", monocode_core::mono::new_id()));
            fs::rename(path, stale)
        };
        if let Err(error) = result
            && error.kind() != io::ErrorKind::NotFound
        {
            eprintln!("Could not clean up a settled reply journal: {error}");
        }
    }
    let mut registry = pending();
    if registry
        .get(&key(store, session))
        .is_some_and(|current| Arc::ptr_eq(current, entry))
    {
        registry.remove(&key(store, session));
    }
    Ok(())
}

fn retry_later(store: Database, session: i64, entry: Entry) {
    std::thread::spawn(move || {
        let mut delay = Duration::from_secs(1);
        loop {
            std::thread::sleep(delay);
            if save_entry(&store, session, &entry).is_ok() {
                break;
            }
            delay = (delay * 2).min(Duration::from_secs(60));
        }
    });
}

pub(crate) fn show_pending(
    store: &Database,
    session: i64,
    rows: &mut Vec<Row>,
) -> Option<&'static str> {
    let entry = pending().get(&key(store, session)).cloned()?;
    let reply = entry.lock().unwrap_or_else(|error| error.into_inner());
    if revision(rows).is_ok_and(|before| before == reply.reply.before) {
        let mut restored = rows.clone();
        if apply(&mut restored, &reply.reply.ranges) {
            *rows = restored;
            return Some(if reply.journaled {
                REPLY_RETAINED
            } else {
                REPLY_IN_MEMORY
            });
        }
    }
    None
}

#[cfg(all(test, unix))]
pub(super) fn forget_pending(store: &Database, session: i64) {
    let existing = pending().get(&key(store, session)).cloned();
    if let Some(existing) = existing {
        let _settled = existing.lock().unwrap();
        pending().remove(&key(store, session));
    }
}

pub(crate) fn recover_journals(store: &Database) {
    let Some(dir) = locked(store).ok().and_then(|db| journal_dir(&db)) else {
        return;
    };
    let Ok(paths) = fs::read_dir(dir) else { return };
    for path in paths.flatten().map(|entry| entry.path()) {
        let Some(session) = path
            .file_stem()
            .filter(|_| path.extension().is_some_and(|ext| ext == "json"))
            .and_then(|name| name.to_str())
            .and_then(|name| {
                let id = match name.split_once('_') {
                    Some((id, nonce))
                        if nonce.len() == 32
                            && nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
                    {
                        id
                    }
                    Some(_) => return None,
                    None => name,
                };
                id.parse::<i64>().ok()
            })
        else {
            continue;
        };
        let Some(reply) = fs::read(&path)
            .ok()
            .and_then(|json| serde_json::from_slice::<Reply>(&json).ok())
            .filter(|reply| reply.session == session)
        else {
            continue;
        };
        let entry = Pending {
            reply,
            journal: Some(path),
            journaled: true,
        };
        let mut registry = pending();
        let std::collections::hash_map::Entry::Vacant(slot) = registry.entry(key(store, session))
        else {
            continue;
        };
        let entry = Arc::new(Mutex::new(entry));
        slot.insert(entry.clone());
        drop(registry);
        if save_pending(store, session).is_err() {
            retry_later(store.clone(), session, entry);
        }
    }
}
