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
    finished_rows: &[Row],
    ranges: &[(usize, &[Row])],
) -> Result<Option<bool>, String> {
    let (reply, journal) = {
        let db = locked(store)?;
        let Some(owner) = owner(&db, session)? else {
            return Ok(None);
        };
        let Some(baseline) = db.reply_recovery_baseline(session)? else {
            return Ok(None);
        };
        let mut saved = baseline.rows;
        if saved.len() > finished_rows.len()
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
        let journal = journal_dir(&db).map(|dir| dir.join(format!("{session}.json")));
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
    pending().insert(key(store, session), Arc::new(Mutex::new(entry)));
    retry_later(store.clone(), session);
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
    fs::create_dir_all(path.parent().expect("journal has a directory"))?;
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
        fs::rename(&partial, path)
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
    let mut reply = entry.lock().unwrap_or_else(|error| error.into_inner());
    if !pending()
        .get(&key(store, session))
        .is_some_and(|current| Arc::ptr_eq(current, &entry))
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
            return Err(error.to_string());
        }
    }
    pending().remove(&key(store, session));
    Ok(())
}

fn retry_later(store: Database, session: i64) {
    std::thread::spawn(move || {
        let mut delay = Duration::from_secs(1);
        loop {
            std::thread::sleep(delay);
            if save_pending(&store, session).is_ok() {
                break;
            }
            delay = (delay * 2).min(Duration::from_secs(60));
        }
    });
}

pub(crate) fn show_pending(store: &Database, session: i64, rows: &mut Vec<Row>) {
    let Some(entry) = pending().get(&key(store, session)).cloned() else {
        return;
    };
    let reply = entry.lock().unwrap_or_else(|error| error.into_inner());
    if revision(rows).is_ok_and(|before| before == reply.reply.before) {
        let mut restored = rows.clone();
        if apply(&mut restored, &reply.reply.ranges) {
            *rows = restored;
        }
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
            .and_then(|name| name.parse::<i64>().ok())
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
        pending().insert(key(store, session), Arc::new(Mutex::new(entry)));
        if save_pending(store, session).is_err() {
            retry_later(store.clone(), session);
        }
    }
}
