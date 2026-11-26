# Handoff: remaining work after PR #22

PR #22 (`feat/agent-overview-tree`) delivers the status core, the Tree layout, progress
summaries, Mono decision cards and the Mono patrol. CI is green. Two audits list what is left:

- `research/audit-tree-summary.md`: Tree layout and progress summary, requirement by requirement
- `research/audit-status-mono.md`: status core and both Mono phases

The remaining work is split into six child tasks. Every task stays in **this branch and PR #22**.
The user wants the whole effort in one PR. Do not open new PRs.

## Tasks and order

| Task | What | Touches | Can run alongside |
|---|---|---|---|
| `11-26-overview-pre-merge-review` | Independent review, read-only, then a findings list | none (read-only) | all |
| `11-26-status-perf-correctness` | Status refresh cost, timers, terminal stale, legacy Mono cards, migration re-run, growth limits | `shell/agent_activity.rs`, `conversation.rs`, `transcript.rs`, `monos/scheduler.rs`, `shell/progress_summary.rs`, core `store/session_activity.rs`, `mono/*` | tree-polish, windows-attention |
| `11-26-tree-polish` | Visual gaps and missing entry points in the Tree | `shell/project_tree*.rs`, `shell/project_rail.rs` (Tree-only paths), `shell.rs` project menu | status-perf, windows-attention |
| `11-26-tree-motion-followups` | Lingering demote and drag edge cases | `shell/project_tree*.rs`, `shell/layout.rs`, `shell/project_rail.rs` | **after** tree-polish (same files) |
| `11-26-windows-attention` | Windows toast, taskbar badge, flash, sound (R6) | `notifications/platform*.rs`, `notifications/delivery.rs`, `sound.rs`, `main.rs`, `Cargo.toml/lock` | status-perf, tree-polish |
| `11-26-overview-test-coverage` | Missing tests and the three-appearance check | tests across the touched modules | **last**, after the others land |

Findings from the review are fixed in whichever task owns the file, or in a short extra commit
by the reviewer when the fix is small and isolated.

## How to work on the shared branch

- Agents running in parallel each use their own worktree on a lane branch cut from
  `feat/agent-overview-tree`, for example `feat/agent-overview-tree-polish`. When the lane is
  done, merge it back into `feat/agent-overview-tree` and push. Do not push lane branches or
  open PRs for them.
- A single agent working alone may commit directly on `feat/agent-overview-tree`.
- Before the first cargo command in a new worktree, seed the build cache (CLAUDE.md,
  "Seed the build cache").
- Commit format: `<type>(native): <summary>`, with a body for `feat` and `fix`. End each commit with
  the session's Co-Authored-By line. Keep one buildable change per commit.

## Build and test on this Windows machine

- The default `stable` here is 1.93 and cannot build vendored gpui (`cold_path`). Use
  `cargo +1.98.1 ...` and `RUSTUP_TOOLCHAIN=1.98.1 npm run native:dev -- --detach`.
- When a Native Dev app is running it locks the exe. Use
  `cargo +1.98.1 ntest -p monocode-gpui --bin monocode-gpui <filter> -- --test-threads=1`, or
  close the app first.
- On Windows, 43 tests already fail on clean `main` (terminals, ssh, symlinks, `gh`, Windows
  paths). A failure counts as yours only if it does not also fail on `main`.
- Clippy on Windows reports existing lints in `terminal.rs`, `automations/editor.rs`,
  `today/note_composer.rs` and `runtime/remote/tests.rs`. Your own files must be clean; CI lints on
  Linux.
- Run tests in one pass per task (CLAUDE.md). See `.trellis/spec/native/testing.md`,
  "Windows gotchas".

## Rules that apply to every task

- No new user settings. Everything is automatic and the same for everyone.
- Colours only through `theme.rs` functions. The Tree must look right with dark glass, dark solid
  and light themes.
- English UI strings.
- Classic and Focused layouts keep their behaviour unless a task says otherwise.
- The design reference is the Ryugi artboard "Monoptah 多任务总览：项目树侧边栏", page 定稿, under
  `C:\dev\monoptah\.ryugi\artifacts\artboard\agent-overview-sidebar\`. That directory is local and
  not in git.
