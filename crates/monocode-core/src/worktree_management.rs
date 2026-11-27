//! Settings worktree guards; removal keeps the branch and commits.
use std::path::{Path, PathBuf};

pub fn removed_worktree(path: &str) -> String {
    format!("removed:{}", crate::files::path_key(path))
}

/// A merge needs a clean registered working copy and evidence for the exact
/// branch head. A linked GitHub pull request covers squash and rebase merges.
pub fn completed_worktree(
    cwd: &Path,
    trees: &[crate::git::Worktree],
    path: &str,
    base: Option<&str>,
    linked_pr: Option<&crate::session_groups::WorkItem>,
) -> Option<String> {
    let main = trees.first().filter(|tree| tree.main && !tree.missing)?;
    if crate::files::path_key(path) == crate::files::path_key(&main.path) {
        return None;
    }
    let linked = trees
        .iter()
        .filter(|tree| !tree.main && crate::files::is_equal_or_inside(path, &tree.path))
        .max_by_key(|tree| tree.path.len());
    let tree = linked.filter(|tree| !tree.missing)?;
    let branch = tree.branch.as_deref()?;
    let base = base
        .filter(|branch| !branch.is_empty() && *branch != "HEAD")
        .or(main.branch.as_deref())?;
    if branch == base || tree.head.is_empty() {
        return None;
    }
    let branch_ref = format!("refs/heads/{branch}");
    let has_commits = crate::git::git(cwd, &["reflog", "show", "--format=%H", &branch_ref])
        .is_ok_and(|log| {
            log.lines()
                .rfind(|line| !line.is_empty())
                .is_some_and(|start| start != tree.head)
        });
    let merged = has_commits
        && [
            format!("refs/heads/{base}"),
            format!("refs/remotes/origin/{base}"),
        ]
        .iter()
        .any(|reference| {
            crate::git::git(cwd, &["merge-base", "--is-ancestor", &tree.head, reference]).is_ok()
        });
    if (!merged && linked_pr.is_none()) || !clean_worktree(tree) {
        return None;
    }
    (merged || linked_pr.is_some_and(|pr| linked_pr_merged(pr, branch, &tree.head, base)))
        .then(|| merged_worktree_key(tree, branch))
}

/// A branch may have been switched or gained a commit or edits since a probe started.
pub fn worktree_archive_evidence_current(path: &str, evidence: &str) -> bool {
    if evidence == removed_worktree(path) {
        return matches!(Path::new(path).try_exists(), Ok(false));
    }
    crate::git::worktrees(Path::new(path)).is_ok_and(|trees| {
        trees.iter().any(|tree| {
            !tree.main
                && !tree.missing
                && crate::files::is_equal_or_inside(path, &tree.path)
                && tree.branch.as_deref().is_some_and(|branch| {
                    merged_worktree_key(tree, branch) == evidence && clean_worktree(tree)
                })
        })
    })
}

fn merged_worktree_key(tree: &crate::git::Worktree, branch: &str) -> String {
    format!(
        "merged:{}:{branch}:{}",
        crate::files::path_key(&tree.path),
        tree.head
    )
}

fn clean_worktree(tree: &crate::git::Worktree) -> bool {
    crate::git::git(
        Path::new(&tree.path),
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--untracked-files=normal",
        ],
    )
    .is_ok_and(|status| status.is_empty())
}

fn linked_pr_merged(
    item: &crate::session_groups::WorkItem,
    branch: &str,
    head: &str,
    base: &str,
) -> bool {
    use crate::codespaces::{GhRunner, SystemGh};
    let Some(item) =
        crate::session_groups::parse_github_work_item(&item.url).filter(|item| item.kind == "pr")
    else {
        return false;
    };
    let args = [
        "pr".into(),
        "view".into(),
        "--json".into(),
        "state,headRefName,headRefOid,baseRefName".into(),
        "--".into(),
        item.url,
    ];
    SystemGh
        .run(&args, None)
        .is_ok_and(|out| out.success && merged_pr_matches(&out.stdout, branch, head, base))
}

fn merged_pr_matches(json: &str, branch: &str, head: &str, base: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json).is_ok_and(|pr| {
        pr["state"] == "MERGED"
            && pr["headRefName"] == branch
            && pr["headRefOid"] == head
            && pr["baseRefName"] == base
    })
}
pub fn default_root(main: &Path) -> PathBuf {
    main.with_file_name(format!(
        "{}-worktrees",
        main.file_name().unwrap_or_default().to_string_lossy()
    ))
}
pub fn status(cwd: &Path, tree: &crate::git::Worktree) -> (bool, Option<bool>, usize) {
    let locked = crate::git::git(cwd, &["worktree", "list", "--porcelain", "-z"])
        .ok()
        .is_some_and(|s| {
            s.split("\0\0").any(|block| {
                block.starts_with(&format!("worktree {}\0", tree.path))
                    && block
                        .split('\0')
                        .any(|v| v == "locked" || v.starts_with("locked "))
            })
        });
    let dirty = crate::git::git(
        Path::new(&tree.path),
        &["status", "--porcelain", "--untracked-files=normal"],
    )
    .ok()
    .map(|s| !s.is_empty());
    let unpushed = crate::git::git(
        Path::new(&tree.path),
        &["rev-list", "--count", "HEAD", "--not", "--remotes"],
    )
    .ok()
    .and_then(|s| s.trim().parse().ok())
    .unwrap_or(0);
    (locked, dirty, unpushed)
}
pub fn check(cwd: &Path, path: &Path) -> Result<crate::git::Worktree, String> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let key = crate::files::path_key(&canonical.to_string_lossy());
    let tree = crate::git::snapshot(cwd)?
        .worktrees
        .into_iter()
        .find(|t| {
            let canonical = Path::new(&t.path)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(&t.path));
            crate::files::path_key(&canonical.to_string_lossy()) == key
        })
        .ok_or("This path is not a registered worktree of this repository")?;
    if tree.main {
        return Err("The main working copy cannot be deleted".into());
    }
    if tree.branch.is_none() {
        return Err("Create a branch for this detached worktree before deleting it.".into());
    }
    if status(cwd, &tree).0 {
        return Err("This worktree is locked. Unlock it in Git before deleting it.".into());
    }
    Ok(tree)
}
pub fn remove(cwd: &Path, path: &Path) -> Result<(), String> {
    let tree = check(cwd, path)?;
    crate::provider::discard_worktree(Path::new(&tree.path));
    crate::git::git(cwd, &["worktree", "remove", "--force", "--", &tree.path]).map(|_| ())
}
/// Deletes a worktree a deleted session left behind, only when nothing is lost:
/// a linked worktree on a branch, not locked, without uncommitted or untracked
/// changes (re-checked by Git, which is run without `--force`). The branch and
/// its commits stay.
pub fn remove_clean(path: &Path) -> Result<(), String> {
    let snapshot = crate::git::snapshot(path)?;
    let main = snapshot
        .worktrees
        .first()
        .ok_or("No main working copy found")?;
    let main = Path::new(&main.path);
    let tree = check(main, path)?;
    if status(main, &tree).1 != Some(false) {
        return Err("This worktree has uncommitted or untracked changes.".into());
    }
    crate::provider::discard_worktree(Path::new(&tree.path));
    // No `--force`: Git itself refuses a worktree that gained changes since the
    // check above, or one holding submodules.
    crate::git::git(main, &["worktree", "remove", "--", &tree.path]).map(|_| ())
}
/// Undoes a worktree `git::create_worktree` just made for a session whose
/// start then failed: the worktree and its fresh `mc/native-` branch (which
/// holds no commits of its own) go, so a failed start leaves nothing behind.
pub fn discard_new(path: &Path) -> Result<(), String> {
    let snapshot = crate::git::snapshot(path)?;
    let main = snapshot
        .worktrees
        .first()
        .ok_or("No main working copy found")?;
    let main = Path::new(&main.path);
    let tree = check(main, path)?;
    let branch = tree
        .branch
        .filter(|b| b.starts_with("mc/native-"))
        .ok_or("Not a worktree made for a native session")?;
    remove(main, path)?;
    crate::git::git(main, &["branch", "-D", "--", &branch])?;
    // The parent folder goes too when it was made for this worktree alone.
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| crate::git::git(dir.path(), args).unwrap();
        git(&["init", "-b", "main"]);
        git(&["config", "user.email", "fixture@example.invalid"]);
        git(&["config", "user.name", "Fixture"]);
        std::fs::write(dir.path().join("file"), "before").unwrap();
        git(&["add", "file"]);
        git(&["commit", "-m", "Initial"]);
        dir
    }
    #[test]
    fn an_empty_or_dirty_branch_is_not_a_completed_worktree() {
        let dir = repo();
        let tree = crate::git::create_named_worktree(dir.path(), "progress").unwrap();
        let check = || {
            let trees = crate::git::worktrees(dir.path()).unwrap();
            completed_worktree(dir.path(), &trees, &tree.path, Some("main"), None)
        };
        assert_eq!(check(), None, "just creating the worktree is not a merge");
        let branch = Path::new(&tree.path);
        std::fs::write(branch.join("file"), "finished").unwrap();
        crate::git::git(branch, &["commit", "-am", "Finish work"]).unwrap();
        assert_eq!(check(), None, "the change is still unique to the worktree");
        crate::git::git(dir.path(), &["merge", "progress"]).unwrap();
        assert!(check().is_some(), "the work is present in main");
        std::fs::write(branch.join("draft"), "unsaved").unwrap();
        assert_eq!(check(), None, "the checkout still has uncommitted work");
        std::fs::remove_file(branch.join("draft")).unwrap();
        remove_clean(branch).unwrap();
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }

    #[test]
    fn a_missing_path_alone_does_not_confirm_a_merge() {
        let dir = repo();
        let tree = crate::git::create_named_worktree(dir.path(), "cleanup").unwrap();
        remove_clean(Path::new(&tree.path)).unwrap();
        let trees = crate::git::worktrees(dir.path()).unwrap();
        assert_eq!(
            completed_worktree(dir.path(), &trees, &tree.path, None, None),
            None
        );
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }

    #[test]
    fn pull_request_evidence_requires_a_merge_of_the_exact_current_head() {
        let merged = r#"{"state":"MERGED","headRefName":"work","headRefOid":"new-head","baseRefName":"main"}"#;
        assert!(merged_pr_matches(merged, "work", "new-head", "main"));
        assert!(!merged_pr_matches(merged, "work", "later-head", "main"));
        assert!(!merged_pr_matches(merged, "other", "new-head", "main"));
        assert!(!merged_pr_matches(merged, "work", "new-head", "release"));
        assert!(!merged_pr_matches(
            &merged.replace("MERGED", "OPEN"),
            "work",
            "new-head",
            "main"
        ));
    }
    #[test]
    fn removal_guards_and_branch_retention() {
        let dir = repo();
        assert!(check(dir.path(), dir.path()).unwrap_err().contains("main"));
        let tree = crate::git::create_named_worktree(dir.path(), "settings-parity").unwrap();
        let path = Path::new(&tree.path);
        std::fs::write(path.join("untracked"), "discard in disposable test").unwrap();
        assert_eq!(status(dir.path(), &tree).1, Some(true));
        crate::git::git(dir.path(), &["worktree", "lock", "--", &tree.path]).unwrap();
        assert!(check(dir.path(), path).unwrap_err().contains("locked"));
        crate::git::git(dir.path(), &["worktree", "unlock", "--", &tree.path]).unwrap();
        remove(dir.path(), path).unwrap();
        assert!(!path.exists());
        assert!(crate::git::git(
            dir.path(),
            &["show-ref", "--verify", "refs/heads/settings-parity"]
        )
        .is_ok());
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }
    #[test]
    fn a_new_session_worktree_is_discarded_with_its_branch() {
        let dir = repo();
        let path = crate::git::create_worktree(dir.path(), "HEAD", 4242).unwrap();
        assert!(path.is_dir());
        discard_new(&path).unwrap();
        assert!(!path.exists());
        assert!(
            crate::git::git(dir.path(), &["branch", "--list", "mc/native-4242-*"])
                .unwrap()
                .trim()
                .is_empty()
        );
        // Only worktrees made for native sessions are touched.
        let named = crate::git::create_named_worktree(dir.path(), "keep-me").unwrap();
        assert!(discard_new(Path::new(&named.path)).is_err());
        assert!(Path::new(&named.path).is_dir());
        remove(dir.path(), Path::new(&named.path)).unwrap();
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }
    #[test]
    fn a_clean_leftover_worktree_is_removed_with_its_branch_kept() {
        let dir = repo();
        let tree = crate::git::create_named_worktree(dir.path(), "session-leftover").unwrap();
        let path = Path::new(&tree.path);

        remove_clean(path).unwrap();

        assert!(!path.exists());
        assert!(crate::git::git(
            dir.path(),
            &["show-ref", "--verify", "refs/heads/session-leftover"]
        )
        .is_ok());
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }
    #[test]
    fn a_leftover_worktree_with_changes_or_the_main_copy_is_kept() {
        let dir = repo();
        let tree = crate::git::create_named_worktree(dir.path(), "session-dirty").unwrap();
        let path = Path::new(&tree.path);
        std::fs::write(path.join("untracked"), "work in progress").unwrap();

        assert!(remove_clean(path).unwrap_err().contains("uncommitted"));
        assert!(path.join("untracked").is_file());
        assert!(remove_clean(dir.path()).unwrap_err().contains("main"));
        assert!(dir.path().join("file").is_file());

        remove(dir.path(), path).unwrap();
        let _ = std::fs::remove_dir(default_root(dir.path()));
    }
    #[test]
    fn detached_commits_cannot_be_orphaned() {
        let dir = repo();
        let path = dir.path().join("detached");
        crate::git::git(
            dir.path(),
            &[
                "worktree",
                "add",
                "--detach",
                path.to_str().unwrap(),
                "HEAD",
            ],
        )
        .unwrap();
        assert!(remove(dir.path(), &path).unwrap_err().contains("detached"));
        crate::git::git(
            dir.path(),
            &["worktree", "remove", "--force", path.to_str().unwrap()],
        )
        .unwrap();
    }
}
