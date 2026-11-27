# Research: R4 Pinned projects in Tree

- **Query**: PINNED heading and `!pinned` filters, drag-reorder with pinned projects, Classic's shared pin ordering
- **Scope**: internal (origin/main 7106f9c1)
- **Date**: 2026-11-27

Paths under `crates/monocode-gpui/src/`.

## Findings

### Tree entries (shell/project_tree.rs:671-704)

```rust
let pinned = |path: &String| workspace.project_rail_pinned.contains(path);
if workspace.projects.iter().any(pinned) {
    entries.push(ProjectEntry::Heading("Pinned"));            // :677
    entries.extend(workspace.projects.iter().filter(|p| pinned(p)).map(project));
}
for group in &workspace.project_rail_groups {               // :680
    entries.push(ProjectEntry::Group(group.id.clone()));
    ... filter(|path| !pinned(path) && project_rail_group(path) == Some(group.id)) // :688
}
entries.push(ProjectEntry::Heading("Projects"));            // :695
... filter(|path| !pinned(path) && project_rail_group(path).is_none())            // :700
```

Order today: PINNED heading + pinned projects (in `workspace.projects` order) -> each group header + its unpinned members -> PROJECTS heading (with filter button and "+" from `render.rs:331-348`) -> unpinned ungrouped projects. A pinned project that is assigned to a group shows only under PINNED.

Consumers of `tree_project_entries`: `tree_rows` (`project_tree.rs:735-759`, `heading => rows.push(TreeRow::Header(heading))` at :757) and `step_tree_project` (:1246-1264, Project: Previous/Next order).

Rendering: `render.rs:256` `TreeRow::Header(ProjectEntry::Heading(name)) => tree_heading(name)`; `tree_heading` (`render.rs:331-348`) adds filter + add buttons only for "Projects".

Project row trailing area (`render.rs:467-582`, `tree_project_row`): chevron, `tree_project_avatar`, name (`flex_1 truncate`), `counts_row(counts)` (:513, `render.rs:1271-1294`), hover-only `tree-project-new-session` "+" (:514-527) and `tree-project-options` "…" (:528-547). No pin glyph today. Pin icons exist: assets `"pin"`, `"pin-off"` (`assets.rs:108-109`).

Group header active state excludes pinned projects (`render.rs:362-369`).

### Shared data and pin toggling

| File:line | What |
|---|---|
| `workspace.rs:188` | `pub project_rail_pinned: HashSet<String>` (persisted, restored at :584) |
| `workspace.rs:841-845` | `project_rail_peers(a,b)`: same pinned-ness AND (pinned OR same group) |
| `workspace.rs:848-876` | `visible_project_order()`: non-Focused = pinned, then each group's unpinned members, then unpinned ungrouped |
| `workspace.rs:879-887` | `project_peers(path)`: Focused = all; else filtered by `project_rail_peers` (used by the project menu's Move submenu, `shell.rs:6600-6631`) |
| `workspace.rs:889-900` | `move_project(path, to)` on the single `workspace.projects` Vec |
| `shell.rs:6570-6598` | Project menu (`rail: true`, which Tree uses at `render.rs:535-538, 558-562`) has "Pin project"/"Unpin project" (`pin`/`pin-off` icon) |
| `shell.rs:6748-6749` | action "pin" -> `toggle_project_rail_pin(&path, cx)` |
| `shell/project_rail.rs:1617-1623` | `toggle_project_rail_pin`: flips set, `persist`, notifies only `projects_frame` (Classic). Tree redraw relies on the subsequent shell render after the menu closes (no explicit `notify_sidebar`) |
| `shell/project_rail.rs:1490-1511` | Classic hover pin button `rail-pin-project` |
| `shell/project_rail.rs:509-575` | Classic `prepare_project_rail`: Monos, "Pinned" heading + pinned projects + Gap, "Groups" + groups, "Projects" + rest (`!pinned && !grouped`) — Classic keeps its own PINNED heading (`project_rail.rs:524`) and must not change |

### Drag reorder interaction (shell/project_rail.rs)

- Tree press: `start_project_rail_drag` (`project_rail.rs:~1925-2015`); Tree branch `self.tree.drag_blocks(&kind, |item| kind != RailDragKind::Project || workspace.project_rail_peers(item, &id))` (:1939-1945). So a pinned project's drag only considers pinned blocks as targets and an unpinned ungrouped project only unpinned ungrouped ones.
- Block extent: `TreeState::drag_blocks` / `drag_block_end` (`project_tree.rs:343-385`) using `ends_block` (`project_tree.rs:187-210`): any `TreeRow::Header(_)` (heading, group, project) ends a project block, so removing the "Pinned" heading still ends the last pinned block at the next project header.
- Commit: `finish_project_rail_drag` (`project_rail.rs:2098-2212`) rewrites only the dragged subset's slots in `workspace.projects` (:2194-2202), then `persist`.
- Offsets: `project_rail_project_offset` (`project_rail.rs:1681-1694`) and `project_rail_project_raised` (:1719-1727) skip group offsets for pinned projects; Tree uses them in `tree_list_row` (`render.rs:192-193`).
- Drag ghost/lift plate rendering is Tree-generic (`render.rs:115-176`).

Smallest seam: `tree_project_entries` (`project_tree.rs:671-704`) — move the pinned projects after `ProjectEntry::Heading("Projects")` instead of under their own heading; plus the pin mark in `tree_project_row` trailing (`render.rs:513-547`). Ordering/drag/persistence data stays in `workspace.project_rail_pinned` and `workspace.projects`.

### Tests

- No Tree test references the PINNED project heading or `project_rail_pinned` (`git grep project_rail_pinned` hits only code in shell.rs, project_rail.rs, project_removal.rs, project_tree.rs, render.rs, workspace.rs; workspace.rs:1579 is a workspace unit test fixture).
- Do not confuse with the session-level "Pinned" section (`TreeRow::Section { name: "Pinned" }`, `project_tree.rs:986-1005`, `render.rs:605-638`), pinned by `only_loose_sessions_determine_whether_worktree_rows_are_needed` (`project_tree.rs:1591-1642`). R4 is about projects only.
- Tree drag tests: `dragging_a_project_block_steps_the_next_one_aside` (`project_tree.rs:2213`), `a_project_whose_header_scrolled_above_the_tree_can_still_step_aside` (:2292), `a_wheel_scroll_retargets_the_tree_drag_without_moving_the_grabbed_header` (:2355), `dropping_a_tree_block_keeps_the_plate_on_the_first_frame` (:2405), `a_held_tree_drag_scrolls_at_both_edges_even_with_reduce_motion` (:2435) — none use pinned projects.

## Caveats / Not Found

- If pinned projects move into the PROJECTS section, they would sit after all group blocks (groups are emitted before the "Projects" heading). Whether pinned-and-grouped projects stay out of their group is current behavior (pinned wins) in both Classic and Tree.
- `toggle_project_rail_pin` does not call `notify_sidebar`; verify the Tree redraws after pin/unpin from the menu.
