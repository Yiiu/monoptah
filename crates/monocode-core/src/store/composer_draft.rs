use super::Store;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::PathBuf};

const PREFIX: &str = "composer-draft:";

pub(super) fn key(id: i64) -> String {
    format!("{PREFIX}{id}")
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ComposerDraft {
    pub text: String,
    pub attachments: Vec<PathBuf>,
}

impl Store {
    pub fn read_composer_draft(&self, id: i64) -> Result<Option<String>, String> {
        self.setting(&key(id))
    }

    /// A delayed composer save can race with deletion. In that case there is
    /// no session left to receive it.
    pub fn write_composer_draft(&self, id: i64, value: &str) -> Result<(), String> {
        let key = key(id);
        if value == "null" {
            return self.delete_setting(&key);
        }
        if self.shared.is_some() {
            let original = self.persisted_session_id(id)?;
            if self.resolve_existing_session_id(&original) == Some(id) {
                self.save_setting(&key, value)?;
            }
            return Ok(());
        }
        self.conn
            .execute(
                "INSERT INTO native_settings(key,value)
                 SELECT ?1,?2 FROM native_sessions WHERE id=?3
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                rusqlite::params![key, value, id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn composer_draft_sessions(&self) -> Result<HashSet<i64>, String> {
        if self.shared.is_some() {
            return Ok(self.shared_composer_draft_sessions(PREFIX));
        }
        let mut statement = self
            .conn
            .prepare("SELECT id FROM native_sessions JOIN native_settings ON key=?1||id")
            .map_err(|error| error.to_string())?;
        let ids = statement
            .query_map([PREFIX], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        ids.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn draft_metadata_and_deletion_work_across_both_stores() {
        for shared in [false, true] {
            let store = if shared {
                Store::open_monocode(Path::new(":memory:"))
            } else {
                Store::open(Path::new(":memory:"))
            };
            let mut store = store.unwrap();
            let session = store.create("codex", "/project").unwrap();
            let draft = ComposerDraft {
                text: "Unsent".into(),
                attachments: vec!["/project/image.png".into()],
            };
            let value = serde_json::to_string(&draft).unwrap();
            store.write_composer_draft(session.id, &value).unwrap();
            assert_eq!(
                store.composer_draft_sessions().unwrap(),
                HashSet::from([session.id])
            );
            assert_eq!(
                store.read_composer_draft(session.id).unwrap().as_deref(),
                Some(value.as_str())
            );
            assert!(store.draft_sessions().unwrap().is_empty());
            assert!(!store.sessions().unwrap()[0].has_messages);
            store.write_composer_draft(session.id, "null").unwrap();
            assert!(store.composer_draft_sessions().unwrap().is_empty());
            store.write_composer_draft(session.id, &value).unwrap();
            store.delete(session.id).unwrap();
            store.write_composer_draft(session.id, &value).unwrap();
            assert!(store.composer_draft_sessions().unwrap().is_empty());
            assert!(store.read_composer_draft(session.id).unwrap().is_none());
        }
    }
}
