# Research: Specs that apply to the Tree density change

- **Query**: spec files under `.trellis/spec/native/` (and guides) relevant to R1-R7, for implement/check manifests
- **Scope**: internal (origin/main 7106f9c1)
- **Date**: 2026-11-27

## Related Specs

| Path | Why |
|---|---|
| `.trellis/spec/native/index.md` | Entry point; native-dev profile commands, "test in one pass", CI on Linux |
| `.trellis/spec/native/component-guidelines.md` | Theme tokens via `theme.rs` (`content`, `accent_color`, `warning_color`, `shell_fill`, `floating_fill`); `icon_button`; `popover`/`Menu`/`AnchorSlot` anchoring; `Disclosure` (Needs you list); Tree rails `step` spacing rule (`tree_row` adds `pb(2)` except Session); Tree reorder rules; "a navigation layout must keep every entry point" (Settings `tree-settings`); Peek/pin layout and the `tree_shortcuts_hide_the_sidebar_and_allow_peek` regression comparing Today/Inbox widths; "Hover needs an element id"; keybinding scoping |
| `.trellis/spec/native/state-and-async.md` | "Background work" pattern (git off the UI thread, `background_executor().spawn` + `cx.spawn`, stale-result guards) for R6 live branch reads; "Agent activity and turn receipts" (`agent_status`, Done/unread semantics, shell-only completion) for R5/R7; "Tree layout, progress, and Mono patrol" (pins/groups/worktree focus use existing workspace data; Tree reads `Shell::progress_summary`) for R4/R5/R6 |
| `.trellis/spec/native/testing.md` | `crate::shell::tests::window(cx)` fixture, `VisualTestContext`, Linux portability (`secondary-` keys, opaque `/work/...` paths, no macOS assumptions), filtered runs |
| `.trellis/spec/native/directory-structure.md` | Where new code goes; core vs GPUI split (any git helper belongs in `monocode-core`) |
| `.trellis/spec/guides/code-reuse-thinking-guide.md` | Reuse shared pieces (`icon_button`, `CloudTip`, `sidebar_options_menu_items`, `shell_task_chip`, `git_cache`) rather than new copies |
| `.trellis/spec/guides/cross-layer-thinking-guide.md` | Persisted-data contracts: Sidebar options fields and `project_rail_pinned` are shared with Focused/Classic and persisted |

Repo rules also in `CLAUDE.md` "Developing the native app" (native-dev aliases, one test pass per task, Linux-portable tests, fix in the shared component).

## Caveats / Not Found

- No spec documents the Tree nav strip, Monos strip or worktree title source specifically; the closest are the Tree bullets in `component-guidelines.md` and `state-and-async.md`.
