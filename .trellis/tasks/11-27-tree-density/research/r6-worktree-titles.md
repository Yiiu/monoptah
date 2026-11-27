# Research: R6 Tree worktree node titles from live git worktree data

- **Query**: `tree_worktree_row`, `tree_branch`, `catalog.worktrees`, `session_working_copy`; where live git worktree data (path + branch) is already loaded/cached in the shell
- **Scope**: internal (origin/main 7106f9c1)
- **Date**: 2026-11-27

Paths under `crates/monocode-gpui/src/` unless noted.

## Findings

### Current title path

| File:line | What |
|---|---|
| `shell/project_tree.rs:73-79` | `TreeRow::Worktree { project, path, branch: Option<String>, open, counts }` (rows are diffed by `PartialEq` in `replace_rows`, :477-506) |
| `shell/project_tree.rs:510-515` | `tree_session_worktree(id, project)`: cloud -> project, else `session_working_copy(id)` |
| `shell/worktrees.rs:29-41` | `session_working_copy`: `session_worktrees[id]` -> loaded chat `settings.worktree_cwd` -> `summary.cwd` |
| `shell/project_tree.rs:589-596` | `catalog.worktrees: HashMap<project, HashMap<path_key(worktree), Counts>>` (skips `remote://`) |
| `shell/project_tree.rs:1022-1068` | `tree_project_rows`: group loose sessions by worktree path; worktree rows only when `by_worktree.len() > 1` (:1033-1047); main checkout first (:1048-1049); `branch = members.filter(!is_cloud).find_map(tree_branch)` (:1053-1056) |
| `shell/project_tree.rs:719-728` | `tree_branch(id)`: loaded `Conversation::branch_name()` else `session_details[id].branch` |
| `conversation/mono.rs:418-420` | `branch_name() -> self.branch.as_deref()` |
| `conversation.rs:1135-1139, 2375-2392` | a loaded conversation sets `branch = git_cache snapshot .current` for its `execution_cwd()` (detached => short sha, see `snapshot` below) |
| `shell.rs:993` | `session_details: HashMap<i64, monocode_core::store::MobileSessionMetadata>` (DB-backed, per PRD empty) |
| `shell/project_tree/render.rs:721-778` | `tree_worktree_row`: `queue_tree_diff(&path)`; `title = branch.clone().unwrap_or_else(|| project_name(&path))` (:733); `tree_node(.., title, "git-branch", ..)`; diff caption; status dot; hover "+" -> `tree_new_session_in_worktree(project, WorktreeFocus { path, branch })` (:734-737, :756-764); click toggles `tree.worktrees` |
| `workspace.rs:1517-1523` | `project_name(path)` = last path component |
| `render.rs:1244-1269` | `tree_node(id, title, glyph, open, openness)`: 25px, title `flex_1 min_w_0 truncate`, `aria_label(title)` — no tooltip, no secondary text slot |
| `shell/project_tree.rs:1177-1192` | `focus_tree_session_worktree` also stores `WorktreeFocus { path, branch: tree_branch(id) }` |

### Live git data structs (crates/monocode-core/src/git.rs)

```rust
// git.rs:17-26
pub struct Worktree {
    pub path: String,          // from `git worktree list --porcelain -z` ("worktree <path>")
    pub branch: Option<String>,// "branch refs/heads/<name>"; None when detached
    pub head: String,          // "HEAD <sha>"
    pub main: bool,            // first entry
    pub missing: bool,         // !Path::is_dir
}
// git.rs:27-34
pub struct Snapshot {
    pub current: Option<String>, // symbolic-ref --short HEAD, else rev-parse --short HEAD (sha)
    pub detached: bool,
    pub linked: bool,
    pub branches: Vec<Branch>,
    pub worktrees: Vec<Worktree>,
}
```

- `git::snapshot(cwd)` (`git.rs:178-231`): rev-parse toplevel, symbolic-ref, `worktrees(cwd)`, canonicalize, `for-each-ref` of all branches (heavier).
- `git::worktrees(cwd)` (`git.rs:233-256`): just `git worktree list --porcelain -z` (cheaper; "Listing working copies does not require reading every local/remote branch"). Remote paths go to `remote::worktrees`.
- Paths from git are forward-slash on Windows (`C:/dev/x`); compare with `monocode_core::files::path_key` (`crates/monocode-core/src/files/paths.rs:40-52`: normalizes slashes, trims trailing `/`, lowercases drive paths).

### Existing caches / loaders

| File:line | What |
|---|---|
| `git_cache.rs:1-73` | Process-global LRU (`LIMIT = 24`) `Mutex<Memory>` of `Snapshot` keyed by `path_key(cwd)`. `get(cwd) -> Option<Snapshot>` (clone, no I/O); `remember(cwd, &Snapshot)`; `load(cwd)` = `git::snapshot` + remember (blocking, call on background executor). `remember` also stores a copy under the **main worktree's path** (`Memory::remember`, :38-54), so any snapshot of a linked worktree warms the project root entry. No change notification. |
| `git_status.rs:933-944` | `ProjectGit::warm_choices`: background `git_cache::load(project)` for the panel's project. Called from `bind` (online again, :365), `set_project` (:448), `changed` (:927-932), `resume` (:913-925, when stats older than 30s) |
| `git_status.rs:559-601` | file watcher / poll on the git dirs triggers `reload` (diff index) only — not `warm_choices` |
| `shell.rs:2195-2217` | `sync_git`: binds `self.git` (`ProjectGit`) to `panel_root(cx).project()` (active project only) |
| `shell.rs:4753-4754` | `resume_git` -> `git.resume(cx)` |
| `conversation.rs:2375-2392` | each loaded conversation `git_cache::load(execution_cwd)` on load/refresh (warms its worktree and, via `remember`, the main checkout) |
| `worktree_switcher.rs:163-182` | switcher flyout: `git_cache::get(cwd)` then background `git_cache::load(cwd)` |
| `worktree_switcher.rs:48-60` | switcher rows read `state.worktrees` (`main` row uses `t.branch`) |
| `git_picker.rs:248-252`, `quick_composer.rs:388-396` | same get-then-load pattern |
| `shell/project_tree/tree_diffs.rs:1-49` | Tree-local per-path async cache pattern: `Diffs { values: HashMap<path,(Instant, Option<DiffStats>)>, pending: HashSet }`; `queue_tree_diff(path)` called from `tree_worktree_row` render, skips under `cfg!(test)`, TTL 20s, background `diff_stats`, then `notify_sidebar` |
| `shell.rs:991` | `worktree_focus: HashMap<project, WorktreeFocus { path, branch }>` (`worktree_switcher.rs:21-25`), persisted; `worktree_renamed` (`shell/worktrees.rs:10-22`, from `shell.rs:1990`) patches its branch on generated-name rename |
| `shell.rs:992` | `session_worktrees: HashMap<i64, Option<String>>` (paths only, no branch) |

No shell-level map from worktree path to current branch exists; the only cached branch-per-worktree data is `git_cache` Snapshots (`.worktrees[*].branch`), populated opportunistically for the active project and for loaded conversations' cwds.

Smallest seams:
- Title: `tree_worktree_row` `title` (`render.rs:733`) and/or the `branch` computed in `tree_project_rows` (`project_tree.rs:1053-1056`, which feeds both `TreeRow::Worktree.branch` and the `WorktreeFocus` passed to the "+" button).
- Data: `git_cache::get(project).and_then(|s| s.worktrees.iter().find(|w| path_key(&w.path) == path_key(&copy)))` gives `branch: Option<String>` (None = detached), `path`, `missing`, `head`. Freshness would need a Tree-side queue like `tree_diffs.rs` (background `git_cache::load` / `git::worktrees` per project with a TTL, then `notify_sidebar`), since nothing else refreshes non-active projects.
- "detached" suffix + full-path hover: `tree_node` has no secondary caption or tooltip; callers append children (e.g. `.child(tree_caption(..))` as the diff caption does at `render.rs:750`) and `Stateful<Div>` accepts `.tooltip(..)` (`CloudTip`/`ProjectTooltip`).

### Tests

- `only_loose_sessions_determine_whether_worktree_rows_are_needed` (`project_tree.rs:1591-1642`), `worktree_hover_button_creates_and_reuses_a_blank_in_that_copy` (:1729-1783, selectors `tree-worktree-{path}`, `tree-worktree-new-session-{path}`), `a_hidden_mono_launch_appears_in_its_worktree_with_the_mascot` (:1785-1852), `collapsing_projects_and_worktrees_keeps_child_rows_aligned` (:2154+, `tree-worktree-/work/another-project`). All match `TreeRow::Worktree { path, .. }` / selectors by path; none assert the title or `branch` field.
- `git_cache.rs:75-124` unit tests (main-checkout warming, LRU bound).
- Tests use fake POSIX paths (`/work/...`) with no git repo, and `queue_tree_diff` is disabled under `cfg!(test)`; a git-backed title test would need `git_cache::remember(..)` seeding (pattern: `shell/worktrees.rs:344`, `worktree_switcher.rs:630`, `git_picker.rs:64`).

## Caveats / Not Found

- `git_cache` is LRU-24 process-wide; many projects/worktrees can evict entries.
- A loaded session in a detached worktree reports `branch` = short sha (from `Snapshot.current`), not None.
- `ProjectGit`'s watcher does not refresh `git_cache`, so a branch switch in the active project is only picked up on `changed`/`resume`/project switch, or by a conversation reload.
- Remote (`remote://`) projects never get worktree rows (`project_tree.rs:589`).
