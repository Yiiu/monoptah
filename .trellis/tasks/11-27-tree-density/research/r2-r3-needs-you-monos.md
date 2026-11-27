# Research: R2 Needs you empty state, R3 Monos roster in Tree

- **Query**: Needs-you empty state, disclosure, Ctrl+J; Tree Mono roster rendering, click, status, avatar
- **Scope**: internal (origin/main 7106f9c1)
- **Date**: 2026-11-27

Paths under `crates/monocode-gpui/src/`.

## R2 Needs you

### Code

| File:line | What |
|---|---|
| `shell/project_tree/render.rs:17-29` | Each render: `catalog = tree_catalog(cx)`; `self.tree.requests = catalog.requests`; `request_disclosure.set(has_requests, has_requests && !cx.reduce_motion(), now)` |
| `shell/project_tree/render.rs:59` | `.child(self.tree_request_queue(&self.tree.requests, cx))` — always a child of the Tree column, between nav and list |
| `shell/project_tree/render.rs:794-844` | `tree_request_queue` |
| `shell/project_tree/render.rs:797-808` | Empty: returns `div().id("tree-needs-you").mx(6).mb(4)` with `debug_selector("tree-needs-you-empty")`, `h(28)`, text "Nothing needs you" in `needs_you_color(0)` = `content(0.4)` |
| `render.rs:809-831` | Non-empty summary row `tree-next-request`: hand icon, "Needs you {n}", caption "Ctrl J" (hardcoded), click `open_next_tree_request` |
| `render.rs:832-843` | List `tree-request-list` (max_h 118, scroll) wrapped in `self.tree.request_disclosure.render(list, now)`; the summary row is always drawn when non-empty; only the list is disclosed |
| `render.rs:1173-1179` | `needs_you_color(count)` |
| `shell/project_tree.rs:265` | `TreeState.request_disclosure: Disclosure` (init `Disclosure::new(false)`, :304) |
| `shell/project_tree.rs:534-555, 599-626` | `catalog.requests`: non-archived, non-deleting sessions with `status.needs_you()` excluding Mono habit sessions; plus Mono notification chats; sorted and deduped |
| `shell/project_tree.rs:1222-1232` | `open_next_tree_request`: returns if not Tree; `cycle(&ids, last, 1)` -> `None` on empty list => no-op |
| `shell.rs:319` | `KeyBinding::new("ctrl-j", NextTreeRequest, Some("Workspace && TreeLayout"))` (literal ctrl on all platforms) |
| `shell.rs:10348-10352` | handler: `if tree_layout() && settings.is_none() { open_next_tree_request }` — independent of whether the queue row is rendered |
| `disclosure.rs:23-83` | `Disclosure::new/set/fraction/animating/mounted/render` (`render` returns `Option<Div>`, None when shut) |

Closing to empty: `set(false, false, ..)` (animate is `has_requests && ..` = false), so it shuts at once; the empty branch returns before touching the disclosure.

Smallest seam: the `ids.is_empty()` branch at `render.rs:797-808` (return an empty element), or skip the `.child(...)` at `render.rs:59` when `self.tree.requests` is empty.

### Tests pinning it

- `needs_you_uses_a_neutral_empty_state_and_discloses_requests` (`shell/project_tree.rs:1558-1589`): asserts `needs_you_color(0) == content(0.4)`, `needs_you_color(1) == warning_color()`, and disclosure mounted/animating per reduce_motion. It does not query the `tree-needs-you-empty` selector (no test does: `git grep tree-needs-you` only finds render.rs).
- `needs_you_opens_the_other_project_and_uses_its_worktree` (`project_tree.rs:1305-1339`), `needs_you_iteration_wraps_and_recovers_if_a_request_has_resolved` (`project_tree.rs:2650`).

## R3 Monos

### Code

| File:line | What |
|---|---|
| `shell/project_tree.rs:58-59, 730-734` | `TreeRow::Monos` pushed first when `workspace.preferences.monos` |
| `shell/project_tree.rs:186-210, 430-434` | `TreeRow::Monos` ends drag blocks / resets owners |
| `shell/project_tree/render.rs:248-255` | `TreeRow::Monos => monos.update(|m, cx| m.tree_rail(tree_section_heading("Monos"), cx))` |
| `render.rs:1181-1200` | `tree_section_heading(name)` (min_h 26, pl 6) + `tree_heading_text` (10px SEMIBOLD uppercase `content(0.45)`) |
| `render.rs:324-328` | every non-session tree row gets `px(6).pb(2)` |
| `monos/mod.rs:1545-1547` | `tree_rail(heading, cx) -> rail_impl(None, false, false, Some(heading), cx)` |
| `monos/mod.rs:1557-1854` | `rail_impl` shared by Focused (`rail`), Original nav (`project_navigation_rail`), and Tree |
| `monos/mod.rs:1565-1567` | returns empty when `!self.prefs.monos` |
| `monos/mod.rs:1568-1575` | roster = all `self.roster.monos` (Tree passes `project: None`) |
| `monos/mod.rs:1614-1628` | heading gets `icon_button("new-mono","New mono","plus")` (selector `sidebar-mono-button`, 20px) only when roster non-empty; heading id/selector `sidebar-mono-heading` |
| `monos/mod.rs:1631` | Tree wrapper `pb_2` |
| `monos/mod.rs:1637-1662` | empty roster: 32px "New mono" row (`new-mono-row`, selector `new-mono`) -> `create(...)` |
| `monos/mod.rs:1663-1846` | one 32px row per Mono (`mono-rail-{id}`, group `mono-rail-row`), opacity 0.65 unless active |
| `monos/mod.rs:1665-1669` | status: `self.chats.get(&mono.id).map(|v| v.read(cx).mono_status()).unwrap_or("idle")` |
| `conversation/mono.rs:407-414` | `mono_status()`: NeedsYou -> "needs-you"; Running/BackgroundWait -> "working"; Done/Idle -> "idle" |
| `monos/mod.rs:1670-1687` | unread replies count = assistant first rows after `mono.seen_rows`; 0 when active |
| `monos/mod.rs:1688-1696` | aria label `"{label}, {state}[, {n} new replies]"` |
| `monos/mod.rs:1729` | `avatar::avatar(&mono.mascot, mono.color, state, 16., true)` |
| `monos/mod.rs:1730-1748` | status dot (working = mono color, needs-you = accent, else `content(0.3)`) only when `original` (not Tree) |
| `monos/mod.rs:1763-1801` | unread dot (6px accent when !original), hidden on hover |
| `monos/mod.rs:1802-1825` | hover "Mono options" button -> `open_options(&id, None, ..)` |
| `monos/mod.rs:1827-1832` | click: `if take(skip_click) return; this.open(&id, w, cx)` |
| `monos/mod.rs:1833-1845` | right-click options; left mouse-down starts Mono drag reorder (`self.drag`) |
| `monos/mod.rs:1576-1578, 1701, 1826` | `rail_bounds[id]` anchor slot probe per row (used by drag/options) |
| `monos/mod.rs:1849-1852` | options surface/background modal painted from the rail when no Mono is open |
| `monos/mod.rs:1398-1413` | `Monos::open(id)`: returns if prefs off/unknown; `make_active`, `ensure_chat`, emit `TabOpened`, focus, mark rows seen, save |
| `monos/avatar.rs:43-46` | `pub(crate) fn avatar(name: &str, color: u32, status: &str, side: f32, _still: bool) -> AnyElement` — canvas pixel sprite; only "working" animates (pulse lease) |
| `shell/project_tree/render.rs:998-1010` | Tree session rows already use `crate::monos::avatar::avatar(&m.mascot, m.color, "idle", 10., true)` |
| `shell/sidebar_list.rs:606-609, 631, 637-640` | Sidebar options `sidebar-monos` toggles `preferences.monos` (+ `apply_preferences`, `sync_settings`) |

Mono fields used: `mono.id`, `mono.label()`, `mono.mascot`, `mono.color`, `mono.seen_rows`, `works_on(project)`.

Smallest seam: `rail_impl` with `tree_heading.is_some()` (`tree` flag at `monos/mod.rs:1585`) — the Tree branch can lay the heading and per-Mono avatars out in one row; or a new `tree_rail` body. Focused and Original call the same `rail_impl`, so any change must stay behind `tree`.

### Tests pinning it

- `only_the_tree_layout_uses_the_tree_mono_heading` (`shell/project_tree.rs:1401-1426`): `sidebar-mono-heading` height Tree 26 / Focused 28 / Classic 30.
- `sidebar_mono_only_marks_unread_replies_and_swaps_the_mark_for_options_on_hover` (`monos/tests.rs:2757+`), `shell/classic_surface_tests.rs:587,695`, `shell/project_rail.rs:2849`, `shell/title_tabs.rs:792`, `shell/session_groups.rs:2633` (`sidebar-mono-button`), `shell/sidebar_list.rs:1205,1254,1281` (`new-mono`): all Focused/Classic (default layout is Focused), unaffected if Tree-gated.

## Caveats / Not Found

- In Tree today the running status dot is not drawn (only `original` draws it); only the avatar's own animation shows "working".
- No tooltip on Mono rows today; label/state is only in `aria_label`.
- Mono rows support drag-reorder and right-click options; a compact avatar strip would need to decide whether those stay.
