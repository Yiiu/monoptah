//! Current git branches for the Tree sidebar's working-copy nodes.
use super::*;
use monocode_core::git::{self, Worktree};
use std::path::Path;

const WORKTREE_TTL: Duration = Duration::from_secs(20);

#[derive(Default)]
pub(super) struct TreeWorktrees {
    values: HashMap<String, CachedWorktrees>,
    // True when invalidation superseded the in-flight read.
    pending: HashMap<String, bool>,
}

struct CachedWorktrees {
    checked: Option<Instant>,
    // None records a failed git lookup. A successful list may omit the session's old path.
    worktrees: Option<Vec<Worktree>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WorktreeTitle {
    Branch(String),
    Directory { name: String, detached: bool },
}

impl WorktreeTitle {
    pub(super) fn branch(&self) -> Option<&str> {
        match self {
            Self::Branch(branch) => Some(branch),
            Self::Directory { .. } => None,
        }
    }
}

impl TreeWorktrees {
    pub(super) fn invalidate(&mut self) {
        for entry in self.values.values_mut() {
            entry.checked = None;
        }
        for stale in self.pending.values_mut() {
            *stale = true;
        }
    }

    fn observe_branch(&mut self, project: &str, path: &str, branch: Option<&str>) {
        let key = path_key(project);
        let copy = path_key(path);
        let matches = self
            .values
            .get(&key)
            .and_then(|entry| entry.worktrees.as_ref())
            .and_then(|trees| trees.iter().find(|tree| path_key(&tree.path) == copy))
            .is_some_and(|tree| {
                tree.branch.as_deref() == branch
                    // The status reader uses a short HEAD for detached checkouts.
                    || (tree.branch.is_none() && branch.is_some_and(|head| tree.head.starts_with(head)))
            });
        if !matches {
            if let Some(entry) = self.values.get_mut(&key) {
                entry.checked = None;
            }
            if let Some(stale) = self.pending.get_mut(&key) {
                *stale = true;
            }
        }
    }

    pub(super) fn title(
        &self,
        project: &str,
        path: &str,
        fallback: impl FnOnce() -> Option<String>,
    ) -> WorktreeTitle {
        let key = path_key(path);
        let find = |worktrees: &[Worktree]| {
            worktrees
                .iter()
                .find(|tree| path_key(&tree.path) == key)
                .map(|tree| tree.branch.clone())
        };
        let branch = match self.values.get(&path_key(project)) {
            Some(entry) => match &entry.worktrees {
                Some(worktrees) => find(worktrees),
                None => Some(None),
            },
            None => crate::git_cache::get(project).and_then(|snapshot| find(&snapshot.worktrees)),
        };
        let detached = matches!(branch, Some(None));
        branch.unwrap_or_else(fallback).map_or_else(
            || WorktreeTitle::Directory {
                name: project_name(path),
                detached,
            },
            WorktreeTitle::Branch,
        )
    }

    pub(super) fn remember(&mut self, project: String, worktrees: Option<Vec<Worktree>>) {
        self.values.insert(
            project,
            CachedWorktrees {
                checked: Some(Instant::now()),
                worktrees,
            },
        );
    }
}

impl Shell {
    pub(in crate::shell) fn invalidate_tree_worktrees(&mut self, cx: &mut Context<Self>) {
        self.tree.worktree_titles.invalidate();
        if self.workspace.tree_layout() {
            self.notify_sidebar(cx);
        }
    }

    pub(in crate::shell) fn observe_tree_git_branch(
        &mut self,
        path: &str,
        branch: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.tree
            .worktree_titles
            .observe_branch(&self.workspace.project, path, branch);
        self.notify_sidebar(cx);
    }

    pub(super) fn queue_tree_worktrees(&mut self, project: &str, cx: &mut Context<Self>) {
        let key = path_key(project);
        let cache = &mut self.tree.worktree_titles;
        if cfg!(test)
            || cache
                .values
                .get(&key)
                .and_then(|entry| entry.checked)
                .is_some_and(|checked| checked.elapsed() < WORKTREE_TTL)
            || cache.pending.contains_key(&key)
        {
            return;
        }
        if let Some(snapshot) = crate::git_cache::get(project) {
            cache.values.entry(key.clone()).or_insert(CachedWorktrees {
                checked: None,
                worktrees: Some(snapshot.worktrees),
            });
        }
        cache.pending.insert(key.clone(), false);
        let project = project.to_owned();
        let work = cx
            .background_executor()
            .spawn(async move { git::worktrees(Path::new(&project)).ok() });
        cx.spawn(async move |this, cx| {
            let worktrees = work.await;
            let _ = this.update(cx, |this, cx| {
                let cache = &mut this.tree.worktree_titles;
                if cache.pending.remove(&key) == Some(false) {
                    cache.remember(key, worktrees);
                }
                // A focus change can supersede the query while it is in flight. In
                // that case, rendering queues a fresh read after the old one ends.
                if this.workspace.tree_layout() {
                    this.notify_sidebar(cx);
                }
            });
        })
        .detach();
    }
}
