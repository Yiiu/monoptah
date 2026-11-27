//! Durable native turn receipts. The shared Desktop database remains untouched.
use super::{shared, SessionLocation, Store};
use crate::agent_status::{Outcome, TurnEnd};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::collections::HashMap;

const BACKFILL_KEY: &str = "session_activity_backfilled_at";
const ACTIVITY_SCHEMA: &str = "(
    session_id TEXT PRIMARY KEY NOT NULL,
    turn_started_at INTEGER,
    owner_pid INTEGER,
    owner_started TEXT,
    turn_ended_at INTEGER,
    outcome TEXT,
    seen_at INTEGER,
    progress_summary TEXT,
    auto_archive_key TEXT
)";
const ACTIVITY_COLUMNS: &str = "session_id,turn_started_at,owner_pid,owner_started,
    turn_ended_at,outcome,seen_at,progress_summary,auto_archive_key";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionActivity {
    pub last_turn: Option<TurnEnd>,
    pub seen_at: Option<u64>,
    pub progress_summary: Option<String>,
    pub auto_archive_key: Option<String>,
}

impl Store {
    fn session_activity_db(&self) -> Result<&Connection, String> {
        if self.uses_monocode_schema() {
            self.shared_writable()?;
        }
        // In-memory shared fixtures do not get a sidecar.
        Ok(self.native_db().unwrap_or(&self.conn))
    }

    /// Call once during Native startup, before accepting new turns or sending prompts.
    pub fn restore_session_activity(
        &self,
        now: u64,
    ) -> Result<HashMap<i64, SessionActivity>, String> {
        let conn = self.session_activity_db()?;
        let tx = sql(Transaction::new_unchecked(
            conn,
            TransactionBehavior::Immediate,
        ))?;
        let existed = shared::has_table(&tx, "session_activity")?;
        let recreated = !activity_schema_intact(&tx)?;
        let had_recovery = shared::has_table(&tx, "session_activity_recovery")?;
        sql(tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS native_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        ))?;
        let backfilled = activity_backfilled_at(&tx)?.is_some();
        let backfill = !existed && !backfilled && !had_recovery;
        let sessions = self.session_activity_sessions(backfill)?;
        if recreated {
            sql(tx.execute_batch(&format!(
                "DROP TABLE IF EXISTS session_activity;
                 CREATE TABLE session_activity {ACTIVITY_SCHEMA};"
            )))?;
        } else {
            add_activity_fields(&tx)?;
        }
        if backfill {
            let mut insert = sql(tx.prepare(
                "INSERT INTO session_activity(session_id,turn_ended_at,outcome,seen_at)
                 VALUES(?1,?2,?3,?4)",
            ))?;
            for (key, (_, history_at)) in &sessions {
                // No completed turn is invented for drafts that have never run.
                sql(insert.execute(params![
                    key,
                    history_at.map(|at| sql_timestamp(at.min(now))),
                    history_at.map(|_| "completed"),
                    sql_timestamp(now),
                ]))?;
            }
        }
        if !backfilled {
            sql(tx.execute(
                "INSERT INTO native_settings(key,value) VALUES(?1,?2)",
                params![BACKFILL_KEY, now.to_string()],
            ))?;
        }
        if recreated && had_recovery {
            sql(tx.execute_batch(&format!(
                "INSERT INTO session_activity ({ACTIVITY_COLUMNS})
                 SELECT {ACTIVITY_COLUMNS} FROM session_activity_recovery;"
            )))?;
        }
        if !had_recovery || recreated {
            install_activity_recovery(&tx, had_recovery)?;
        }
        self.recover_session_turns(&tx, &sessions, now)?;

        let mut result = HashMap::new();
        {
            let mut query = sql(
                tx.prepare("SELECT session_id,turn_ended_at,outcome,seen_at,progress_summary,auto_archive_key FROM session_activity")
            )?;
            let mut rows = sql(query.query([]))?;
            while let Some(row) = sql(rows.next())? {
                let key: String = sql(row.get(0))?;
                let Some(&(id, _)) = sessions.get(&key) else {
                    continue;
                };
                let end = sql(row.get::<_, Option<i64>>(1))?.and_then(unsigned_timestamp);
                let outcome: Option<String> = sql(row.get(2))?;
                let last_turn = end
                    .zip(outcome.as_deref().and_then(parse_outcome))
                    .map(|(at, outcome)| TurnEnd { at, outcome });
                result.insert(
                    id,
                    SessionActivity {
                        last_turn,
                        seen_at: sql(row.get::<_, Option<i64>>(3))?.and_then(unsigned_timestamp),
                        progress_summary: sql(row.get(4))?,
                        auto_archive_key: sql(row.get(5))?,
                    },
                );
            }
        }
        sql(tx.commit())?;
        Ok(result)
    }

    /// Recreate a missing or incomplete table before retrying the failed write.
    pub fn repair_session_activity(&self, now: u64) -> Result<bool, String> {
        let conn = self.session_activity_db()?;
        if activity_schema_intact(conn)?
            && activity_has_column(conn, "progress_summary")?
            && activity_has_column(conn, "auto_archive_key")?
        {
            return Ok(false);
        }
        self.restore_session_activity(now)?;
        Ok(true)
    }

    pub fn session_activity_backfilled_at(&self) -> Result<Option<u64>, String> {
        let conn = self.session_activity_db()?;
        if !shared::has_table(conn, "native_settings")? {
            return Ok(None);
        }
        activity_backfilled_at(conn)
    }

    /// The caller owns a local turn. Opening a terminal or mirroring a remote
    /// running indicator does not acquire ownership of that process.
    pub fn begin_session_turn(&self, id: i64, started_at: u64) -> Result<(), String> {
        if id < 0 || !matches!(self.cloud_location(id)?, SessionLocation::Local) {
            return Ok(());
        }
        let conn = self.session_activity_db()?;
        let Some(key) = self.session_activity_key(id)? else {
            return Ok(());
        };
        let pid = std::process::id();
        let Owner::Alive(stamp) = process_owner(pid) else {
            return Err("Cannot identify the process that owns this turn".into());
        };
        sql(conn.execute(
            "INSERT INTO session_activity(session_id,turn_started_at,owner_pid,owner_started)
             VALUES(?1,?2,?3,?4)
             ON CONFLICT(session_id) DO UPDATE SET
                turn_started_at=excluded.turn_started_at,owner_pid=excluded.owner_pid,
                owner_started=excluded.owner_started
             WHERE excluded.turn_started_at>COALESCE(session_activity.turn_started_at,-1)",
            params![key, sql_timestamp(started_at), pid, stamp],
        ))?;
        Ok(())
    }

    /// Complete that generation once, even if a delayed beginner or completion
    /// reaches the writer after a newer turn. Remote and terminal turns can
    /// report a completion without first claiming local ownership.
    pub fn finish_session_turn(
        &self,
        id: i64,
        started_at: u64,
        end: TurnEnd,
    ) -> Result<(), String> {
        self.finish_session_turn_with_summary(id, started_at, end, None)
    }

    pub fn finish_session_turn_with_summary(
        &self,
        id: i64,
        started_at: u64,
        end: TurnEnd,
        summary: Option<&str>,
    ) -> Result<(), String> {
        let conn = self.session_activity_db()?;
        let Some(key) = self.session_activity_key(id)? else {
            return Ok(());
        };
        sql(conn.execute(
            "INSERT INTO session_activity(session_id,turn_started_at,turn_ended_at,outcome,progress_summary)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(session_id) DO UPDATE SET
                turn_started_at=excluded.turn_started_at,turn_ended_at=excluded.turn_ended_at,
                outcome=excluded.outcome,progress_summary=excluded.progress_summary,
                owner_pid=NULL,owner_started=NULL
             WHERE excluded.turn_started_at>COALESCE(session_activity.turn_started_at,-1)
                OR (excluded.turn_started_at=session_activity.turn_started_at
                    AND session_activity.owner_pid IS NOT NULL)",
            params![
                key,
                sql_timestamp(started_at),
                sql_timestamp(end.at.max(started_at)),
                outcome_tag(end.outcome),
                summary
            ],
        ))?;
        Ok(())
    }

    /// Apply model prose only to the latest completed turn. A local turn already started
    /// in another window also supersedes the previous summary job.
    pub fn update_progress_summary(
        &self,
        id: i64,
        ended_at: u64,
        summary: &str,
    ) -> Result<bool, String> {
        let conn = self.session_activity_db()?;
        let Some(key) = self.session_activity_key(id)? else {
            return Ok(false);
        };
        sql(conn.execute(
            "UPDATE session_activity SET progress_summary=?3
             WHERE session_id=?1 AND turn_ended_at=?2 AND owner_pid IS NULL",
            params![key, sql_timestamp(ended_at), summary],
        ))
        .map(|updated| updated != 0)
    }

    /// A manual restore can keep a session visible after the same evidence is seen again.
    pub fn remember_worktree_archive(&self, id: i64, evidence: &str) -> Result<(), String> {
        let conn = self.session_activity_db()?;
        let Some(key) = self.session_activity_key(id)? else {
            return Ok(());
        };
        sql(conn.execute(
            "INSERT INTO session_activity(session_id,auto_archive_key) VALUES(?1,?2)
             ON CONFLICT(session_id) DO UPDATE SET auto_archive_key=excluded.auto_archive_key",
            params![key, evidence],
        ))?;
        Ok(())
    }

    /// Acknowledgements are watermarks for the completed turn the user
    /// saw. Using the input timestamp alone could mark the next turn as seen
    /// if two writes reach the store in a different order.
    pub fn see_session_turn(
        &self,
        id: i64,
        turn_ended_at: u64,
        seen_at: u64,
    ) -> Result<(), String> {
        let conn = self.session_activity_db()?;
        let Some(key) = self.session_activity_key(id)? else {
            return Ok(());
        };
        sql(conn.execute(
            "INSERT INTO session_activity(session_id,seen_at) VALUES(?1,?2)
             ON CONFLICT(session_id) DO UPDATE SET seen_at=excluded.seen_at
             WHERE excluded.seen_at>COALESCE(session_activity.seen_at,-1)",
            params![key, sql_timestamp(turn_ended_at.min(seen_at))],
        ))?;
        Ok(())
    }

    fn session_activity_sessions(
        &self,
        include_history: bool,
    ) -> Result<HashMap<String, (i64, Option<u64>)>, String> {
        let history = if !include_history {
            "0".into()
        } else if self.uses_monocode_schema() {
            self.shared_field(
                "has_user_message",
                "CASE WHEN blocks_json!='[]' THEN 1 ELSE 0 END",
            )
        } else {
            "EXISTS(SELECT 1 FROM native_rows WHERE session_id=native_sessions.id)".into()
        };
        let table = if self.uses_monocode_schema() {
            "sessions"
        } else {
            "native_sessions"
        };
        let mut query = sql(self.conn.prepare(&format!(
            "SELECT CAST(id AS TEXT),{history},updated_at FROM {table}"
        )))?;
        let mut rows = sql(query.query([]))?;
        let mut sessions = HashMap::new();
        while let Some(row) = sql(rows.next())? {
            let key: String = sql(row.get(0))?;
            let id = if self.uses_monocode_schema() {
                self.shared_register(&key)?
            } else {
                key.parse::<i64>().map_err(|error| error.to_string())?
            };
            let has_history = sql(row.get::<_, bool>(1))?;
            let history_at = has_history
                .then(|| sql(row.get::<_, i64>(2)))
                .transpose()?
                .map(|at| at.max(0) as u64);
            sessions.insert(key, (id, history_at));
        }
        for (id, remote) in self.remote_sessions()? {
            if id < 0 {
                sessions.insert(
                    remote.persisted_key(id),
                    (
                        id,
                        remote
                            .summary
                            .has_messages
                            .then_some(remote.summary.updated_at),
                    ),
                );
            }
        }
        Ok(sessions)
    }

    pub(super) fn session_activity_key(&self, id: i64) -> Result<Option<String>, String> {
        if id < 0 {
            return Ok(self
                .remote_session(id)?
                .map(|remote| remote.persisted_key(id)));
        }
        let Ok(key) = self.persisted_session_id(id) else {
            return Ok(None);
        };
        let exists = if self.uses_monocode_schema() {
            sql(self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
                [&key],
                |row| row.get::<_, bool>(0),
            ))?
        } else {
            sql(self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM native_sessions WHERE id=?1)",
                [id],
                |row| row.get::<_, bool>(0),
            ))?
        };
        Ok(exists.then_some(key))
    }

    pub(super) fn clear_session_activity(&self, key: &str) -> Result<(), String> {
        let conn = self.session_activity_db()?;
        if shared::has_table(conn, "session_activity")? {
            sql(conn.execute("DELETE FROM session_activity WHERE session_id=?1", [key]))?;
        }
        if shared::has_table(conn, "session_activity_recovery")? {
            sql(conn.execute(
                "DELETE FROM session_activity_recovery WHERE session_id=?1",
                [key],
            ))?;
        }
        Ok(())
    }

    fn recover_session_turns(
        &self,
        tx: &Transaction<'_>,
        sessions: &HashMap<String, (i64, Option<u64>)>,
        now: u64,
    ) -> Result<(), String> {
        // Desktop and cloud may still own their work. Only Native's own
        // abandoned local turns have enough evidence to be marked interrupted.
        let open = {
            let mut query = sql(tx.prepare(
                "SELECT session_id,owner_pid,owner_started,turn_started_at,seen_at
                 FROM session_activity WHERE owner_pid IS NOT NULL",
            ))?;
            let rows = sql(query.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            }))?;
            sql(rows.collect::<rusqlite::Result<Vec<_>>>())?
        };
        let mut owners = HashMap::new();
        for (key, pid, stamp, started_at, seen_at) in open {
            let Some(&(id, _)) = sessions.get(&key) else {
                continue;
            };
            if id < 0 || !matches!(self.cloud_location(id), Ok(SessionLocation::Local)) {
                continue;
            }
            if owners
                .entry(pid)
                .or_insert_with(|| process_owner(pid))
                .matches(stamp.as_deref())
            {
                continue;
            }
            let ended_at = sql_timestamp(now)
                .max(started_at)
                .max(seen_at.map_or(0, |at| at.saturating_add(1)));
            sql(tx.execute(
                "UPDATE session_activity SET
                    turn_ended_at=?2,outcome='interrupted',progress_summary='Stopped before finishing',
                    owner_pid=NULL,owner_started=NULL
                 WHERE session_id=?1",
                params![key, ended_at],
            ))?;
        }
        Ok(())
    }
}

fn activity_schema_intact(conn: &Connection) -> Result<bool, String> {
    sql(conn.query_row(
        "SELECT count(*) = 7 FROM pragma_table_info('session_activity')
         WHERE name IN ('session_id','turn_started_at','owner_pid','owner_started',
                        'turn_ended_at','outcome','seen_at')",
        [],
        |row| row.get(0),
    ))
}

fn add_activity_fields(tx: &Transaction<'_>) -> Result<(), String> {
    for column in ["progress_summary", "auto_archive_key"] {
        if !activity_has_column(tx, column)? {
            sql(tx.execute(
                &format!("ALTER TABLE session_activity ADD COLUMN {column} TEXT"),
                [],
            ))?;
        }
    }
    Ok(())
}

fn activity_has_column(conn: &Connection, column: &str) -> Result<bool, String> {
    sql(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('session_activity') WHERE name=?1)",
        [column],
        |row| row.get(0),
    ))
}

fn activity_backfilled_at(conn: &Connection) -> Result<Option<u64>, String> {
    let value: Option<String> = sql(conn
        .query_row(
            "SELECT value FROM native_settings WHERE key=?1",
            [BACKFILL_KEY],
            |row| row.get(0),
        )
        .optional())?;
    value
        .map(|at| at.parse::<u64>().map_err(|error| error.to_string()))
        .transpose()
}

fn install_activity_recovery(tx: &Transaction<'_>, existed: bool) -> Result<(), String> {
    // Keep one authoritative recovery row per session. A live repair must
    // retain the owner, read watermarks and summaries the Shell already uses.
    // Triggers keep the copy atomic with all ordinary writes and deletion.
    if !existed {
        sql(tx.execute_batch(&format!(
            "CREATE TABLE session_activity_recovery {ACTIVITY_SCHEMA};
             INSERT INTO session_activity_recovery ({ACTIVITY_COLUMNS})
             SELECT {ACTIVITY_COLUMNS} FROM session_activity;"
        )))?;
    }
    for (name, event) in [("insert", "INSERT"), ("update", "UPDATE")] {
        sql(tx.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS session_activity_recovery_{name} AFTER {event} ON session_activity
             BEGIN
                DELETE FROM session_activity_recovery WHERE session_id=NEW.session_id;
                INSERT INTO session_activity_recovery ({ACTIVITY_COLUMNS})
                SELECT {ACTIVITY_COLUMNS} FROM session_activity WHERE session_id=NEW.session_id;
             END;"
        )))?;
    }
    sql(tx.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS session_activity_recovery_delete AFTER DELETE ON session_activity
         BEGIN DELETE FROM session_activity_recovery WHERE session_id=OLD.session_id; END;",
    ))
}

fn sql<T>(result: rusqlite::Result<T>) -> Result<T, String> {
    result.map_err(|error| error.to_string())
}

fn sql_timestamp(at: u64) -> i64 {
    at.min(i64::MAX as u64) as i64
}

fn unsigned_timestamp(at: i64) -> Option<u64> {
    u64::try_from(at).ok()
}

fn outcome_tag(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Completed => "completed",
        Outcome::Failed => "failed",
        Outcome::Interrupted => "interrupted",
    }
}

fn parse_outcome(tag: &str) -> Option<Outcome> {
    match tag {
        "completed" => Some(Outcome::Completed),
        "failed" => Some(Outcome::Failed),
        "interrupted" => Some(Outcome::Interrupted),
        _ => None,
    }
}

/// When the operating system cannot read an existing owner's birth time,
/// leave the turn to that owner. A known different stamp means PID reuse.
enum Owner {
    Gone,
    Alive(Option<String>),
}

impl Owner {
    fn matches(&self, stamp: Option<&str>) -> bool {
        match self {
            Self::Gone => false,
            Self::Alive(current) => current.as_deref().zip(stamp).is_none_or(|(a, b)| a == b),
        }
    }
}

#[cfg(unix)]
fn process_owner(pid: u32) -> Owner {
    let Ok(raw) = i32::try_from(pid) else {
        return Owner::Gone;
    };
    if raw == 0
        || (unsafe { libc::kill(raw, 0) } != 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH))
    {
        return Owner::Gone;
    }
    Owner::Alive(crate::process::guard::start_stamp(pid))
}

#[cfg(windows)]
fn process_owner(pid: u32) -> Owner {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    if pid == 0 {
        return Owner::Gone;
    }
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return if std::io::Error::last_os_error().raw_os_error()
                == Some(ERROR_INVALID_PARAMETER as i32)
            {
                Owner::Gone
            } else {
                Owner::Alive(None)
            };
        }
        let mut exit_code = 0;
        let exited =
            GetExitCodeProcess(process, &mut exit_code) != 0 && exit_code != STILL_ACTIVE as u32;
        let mut created = FILETIME::default();
        let mut exited_at = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let have_stamp = GetProcessTimes(
            process,
            &mut created,
            &mut exited_at,
            &mut kernel,
            &mut user,
        ) != 0;
        CloseHandle(process);
        if exited {
            Owner::Gone
        } else {
            Owner::Alive(
                have_stamp.then(|| format!("{}:{}", created.dwHighDateTime, created.dwLowDateTime)),
            )
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn process_owner(pid: u32) -> Owner {
    if pid == std::process::id() {
        Owner::Alive(None)
    } else {
        Owner::Gone
    }
}

#[cfg(test)]
#[path = "session_activity_tests.rs"]
mod tests;
