# Implement: Tree polish

Work on lane branch `feat/agent-overview-tree-polish`, cut from `feat/agent-overview-tree`. `11-26-tree-motion-followups` touches the same files and must start **after** this lane is merged back.

Make one commit per step. Finish all edits for a step before running its checks.

1. **R4: single-checkout counting.** In `shell/project_tree.rs`, compute the worktree-level decision from the same filtered session set that builds the worktree rows. Test it in `shell::project_tree::tests`.
2. **R1: Needs-you 0 styling.** Update `render.rs` in two places: the queue, and the status-bar chip. Animate the height with the existing `Disclosure` and its reduce-motion handling. Test it in `project_tree`.
3. **R2: heading style.** The Tree renders the Mono roster heading through a tree heading; Focused keeps `section_label`. Find the call site at `monos/mod.rs:1591` and pass the style from the Tree path. Test both layouts.
4. **R3: group header.** Add a Tree-specific group header in `shell/project_tree/render.rs` instead of reusing `project_rail_group_header`, and route the toggle through `tree_disclose`. Add a Classic regression test that asserts the header height and style are unchanged.
5. **R5: Codespace entry.** In `shell.rs`, the Tree project menu (around 6526–6660) gets `new-codespace-session` when `offers_codespace()` is true, sharing the handler at `shell.rs:6700`. Test it with a stubbed codespace capability, if the existing tests provide one. Otherwise test the menu item list.
6. **R6: worktree "+".** Add a hover button on the worktree row (`render.rs:543-574`). It calls a `tree_new_session_in_worktree(path, worktree)` that reuses `tree_new_session`'s blank check, keyed by worktree. Test it the same way as `a_project_row_starts_a_session_from_its_hover_button`.
7. **R7: peek badge plate.** Pass the plate colour into the row badge when the tree draws inside the peek. Check it visually.

After all steps, run these checks once:

```bash
cargo fmt --all
cargo +1.98.1 nclippy -p monocode-gpui --all-targets -- -D warnings   # judge your files only
cargo +1.98.1 ntest -p monocode-gpui --bin monocode-gpui -- shell::project_tree shell::project_rail monos:: --test-threads=1
```

Then start `RUSTUP_TOOLCHAIN=1.98.1 npm run native:dev -- --detach` and take screenshots of the Tree in three appearances: dark glass, dark solid and light. Attach them to the hand-over.

Hand-over: a checklist that maps R1–R7 to `file:line`, its test and its screenshot. Then merge the lane back and push.
