# Implement: Tree motion follow-ups

Work on the lane branch `feat/agent-overview-tree-motion`, cut from `feat/agent-overview-tree` **after the tree-polish lane has merged**.

## Code map

- Lingering and fades: `TreeState::lingering` and `fades`, together with `tree_note_unread` and `tree_settle_unread` in `shell/project_tree.rs` (around 680–760). Settle is currently called from `render.rs` (tree and status bar).
- Drag: `TreeState::drag_blocks` (`project_tree.rs` ~289, ~345), `ProjectRailDrag::follow` / `step_aside` (`project_rail.rs` ~124/146), `tree_lift_bounds` (`project_rail.rs` ~1638), and the scroll handler at `shell.rs` ~6798.

## Steps

1. **R1.** Add `settle_tree_unread_now()`, which clears `lingering` and `fades` without animating. Call it from `set_navigation_layout` when the layout leaves Tree, and make the three direct `classic_layout` writers go through `set_navigation_layout`. Test with a layout round-trip.
2. **R2.** Replace the `workspace.active` check in settle with `tree_row_in_view(id)`, which is active, with no app view, no app search and no Mono active. Make sure settle runs when an app view opens: notify the sidebar, or call settle from `open_app_view`. Test by opening Inbox.
3. **R3.**
   - Rebuild block geometry from the row model plus list measurements, so blocks above the viewport are included.
   - Re-anchor offsets on scroll during a drag by storing the scroll offset at press time and applying the delta.
   - Edge auto-scroll uses the same frame loop as the drag.
   - On the drop frame, keep the lift plate.
   
   Tests: an above-viewport block, a scroll mid-drag, and the drop frame.

## Checks (run once, at the end)

```bash
cargo fmt --all
cargo +1.98.1 nclippy -p monocode-gpui --all-targets -- -D warnings   # judge your files only
cargo +1.98.1 ntest -p monocode-gpui --bin monocode-gpui -- shell::project_tree shell::project_rail shell::layout --test-threads=1
```

Then do a manual pass in Native Dev: a layout round-trip, opening Inbox, and dragging with a scroll and at the edges.

## Hand-over

A checklist that maps R1–R3 to `file:line` and its test. Merge back and push.
