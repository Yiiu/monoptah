# Research: Audit of the Tree layout and progress summary against the PRDs

- **Query**: Classify every requirement and acceptance criterion of the parent PRD (sections 侧边栏 / Tree 布局的功能对齐 / 进展摘要 / Acceptance Criteria), `11-25-project-tree-sidebar` and `11-25-progress-summary` as DONE / PARTIAL / MISSING / CHANGED, with file:line evidence. Also re-check review follow-ups (a)–(g).
- **Scope**: internal, read-only audit
- **Date**: 2026-11-26
- **Tree**: `.worktrees/agent-status-core`, branch `feat/agent-overview-tree` (commits `a2841072`, `4f4a1fd5`), clean working tree.

Paths below are relative to `crates/monocode-gpui/src/` unless they start with `monocode-core/` (= `crates/monocode-core/src/`).
`tree.rs` = `shell/project_tree.rs`, `render.rs` = `shell/project_tree/render.rs`.

## 1. Layout switching, tabs, right panel

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 1.1 | Settings > Appearance > Layout offers Classic / Focused / Tree | DONE | `settings/pages/appearance.rs:281-293` (3-way segmented, `NavigationLayout::from_id`); `workspace.rs` `NavigationLayout` enum + `id/from_id/next`; `shell/settings.rs` `SettingsEvent::NewLayout(layout) => set_navigation_layout` |
| 1.2 | Ctrl/Cmd+Shift+L and menu "Toggle Layout" cycle Classic → Focused → Tree | DONE | `shell/layout.rs:174-176` `toggle_layout` → `navigation_layout().next()`; `shell.rs:322` keybinding; `app_menu.rs:117`; test `shell/layout.rs` `tree_uses_the_layout_shortcut_and_retains_a_collapsed_status_bar`; `workspace.rs` test `navigation_cycles_and_restores_existing_layouts` |
| 1.3 | No in-sidebar switcher | DONE | `render.rs:5-63` has none |
| 1.4 | Default layout unchanged; "try the new layout" notice does not recommend Tree | DONE | `workspace.rs` `#[default] Focused`; `shell/notifications.rs:207-209` sets Focused |
| 1.5 | Existing two layouts unchanged | DONE (with intended deltas) | Tree state stored in a new `tree_sidebar` flag, `classic_layout` semantics kept (`workspace.rs` `set_navigation_layout`). Intended shared deltas: Focused tab tile now uses the new status icon (`shell.rs:8513`), session status labels in all layouts come from `agent_status` |
| 1.6 | Tree keeps Focused's tabs and right panel; both scoped to current project + worktree | DONE | Tree only swaps `sidebar_contents` (`shell.rs:7718-7720`); tabs/right panel untouched |
| 1.7 | Clicking a session of another project/worktree switches project + worktree (tabs and right panel follow) | DONE | `tree.rs:1077-1092` `focus_tree_session_worktree`, `tree.rs:1094-1120` `open_tree_session`; click path `shell/session_groups.rs:510-511`; tests `tree.rs:1206` and `shell/title_tabs.rs` `tree_opening_a_cloud_twin_keeps_the_local_tab_scope` |
| 1.8 | Ctrl J performs the same switch | DONE | `shell.rs:317` `ctrl-j` in `Workspace && TreeLayout`; `shell.rs:10273-10277`; `tree.rs:1122-1132` |

## 2. Mono in the Tree

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 2.1 | Mono roster at top of tree (below Needs-you, above projects), same component and animations | DONE | `tree.rs:690-692` `TreeRow::Monos` first list row; queue is above the list (`render.rs:44`); `render.rs:170-173` uses `monos.rail(None)` |
| 2.2 | Mono-launched session rows carry the Mono's 10 px pixel avatar | DONE | `render.rs:783-787,817` |
| 2.3 | Subagent pixel avatars inside the conversation unaffected | DONE | not touched by tree code |
| 2.4 | Mono-launched sessions listed in the tree by default | DONE | `tree.rs:495-501` (`launched_by_mono` forces `listed`) |

## 3. Structure rules (project → worktree → session)

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 3.1 | Status computed per agent row | DONE | `tree.rs:493-555` per-session `agent_status` |
| 3.2 | Worktree row shows only the most urgent dot + dim diff (+/−) | DONE | `render.rs:543-574` (`tree_diffs.display`, `counts.color()` dot); `shell/project_tree/tree_diffs.rs` |
| 3.3 | Worktree level hidden when the project has only its main checkout | PARTIAL | `tree.rs:922-936` shows worktree rows only when `catalog.worktrees[path].len() > 1`. Missing piece: that count (`tree.rs:547-554`) includes sessions that are placed in Pinned / Reminders / folders, while the rows are built from `split.loose` only (`tree.rs:940-948`). A project whose only worktree session is pinned therefore gets a lone "main" worktree row |
| 3.4 | Remote projects have no worktree layer | DONE | `tree.rs:547` skips `remote://`; New worktree hidden for remote (`shell.rs:6543`) |
| 3.5 | No worktree switcher in Tree; New worktree in the project row menu | DONE | `shell.rs:6538-6551` adds "New session" + "New worktree" for `rail && tree_layout()`; `shell.rs` run_menu `tree-new-worktree` → `compose_in_new_worktree` |
| 3.6 | Worktree "+" (hover button on a worktree row) | MISSING | `render.rs:543-574` worktree row has no button; only project rows have a hover "+" (`render.rs:325-338`). Not in the PRD text or the artboard either; listed here because the caller asked |
| 3.7 | Removed worktrees auto-archive | DONE (Tree only) | `shell/settings.rs:731` → `progress_summary.rs:327-341`; Git scan every 75 s `progress_summary.rs:175-324`; both gated on `tree_layout()` (`progress_summary.rs:239,348`) as the 2b design specifies |
| 3.8 | Order inside a project: Pinned → Reminders → folders → worktrees | DONE | `tree.rs:886-968`; split logic `shell/project_tree/sections.rs` |
| 3.9 | Sessions in Pinned/Reminders/folders not repeated under worktrees; branch label on those rows | DONE | `sections::sections` dedups; `show_branch = true` for those (`tree.rs:903,919`), `false` under worktrees (`tree.rs:927-934,966`); rendered `render.rs:819` |
| 3.10 | Sessions in a worktree sorted by recent activity, no date subheadings | CHANGED (sanctioned by design.md) | Default sort `tree.rs:574-582`; additionally manual drag order is remembered per scope (`tree.rs:980-996`, `workspace.rs` `tree_session_order`), as 2a design.md allows |
| 3.11 | Orchestrator children and Operator-started sessions as a collapsed 4th level showing only a count | DONE | `tree.rs:587-613` (flattened chains), `tree.rs:997-1041` (`open` only when toggled or parent not matching a filter), count chip `render.rs:826-861` |
| 3.12 | Archive collected at the bottom ("Archived N"), excluded from counts | DONE | `tree.rs:536-541,708-717`; `render.rs:223-241`; test `tree.rs:1356` |

## 4. Needs-you queue, counts, density

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 4.1 | Permanent "Needs you" queue across all projects | DONE | `render.rs:590-630`; catalog `tree.rs:502-513,557-573` |
| 4.2 | Queue collapses to one line at 0 | DONE (see follow-up f) | `render.rs:620` list only when non-empty |
| 4.3 | Ctrl J opens next waiting session | DONE | `tree.rs:1122-1132`, wraps via `cycle` (test `tree.rs:1819`) |
| 4.4 | Project row tri-color counts (needs you / running incl. background wait / unread done), hidden at 0, smaller and dimmer than the name | DONE | `tree.rs:27-48`; `render.rs:1231-1254` (11 px, `content(0.56)` vs name 13 px `content(0.92)`) |
| 4.5 | Counts still visible when the project is collapsed | DONE | counts come from catalog regardless of open state (`render.rs:283,324`); test `tree.rs:1356` |
| 4.6 | Running project → avatar with blue ring | DONE | `render.rs:404-407` (shown whether expanded or collapsed) |
| 4.7 | Row density: needs-you and unread done = 2 lines, others 1 | DONE | `render.rs:790,880-900` |
| 4.8 | Running row: step + duration, ~15 s refresh | DONE | `monocode-core/agent_status.rs:138-148`; `shell/live_agents.rs:8` `STATUS_TICK = 15s` |
| 4.9 | Background wait: "waiting N subtasks", no spinner | DONE | `agent_status.rs:149-155`; no ring (`is_animating` only for `Running`) |

## 5. Visual spec and theme

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 5.1 | List padding 6 px | DONE | `render.rs:243-247` `.px(px(6.))` per row |
| 5.2 | 2 px between rows | CHANGED | Non-session rows: `pb(2)` (`render.rs:245`). Session rows get spacing from `rails::step` `py(2)` (`shell/rails.rs:72-82`), i.e. 2 px above + 2 px below = 4 px between consecutive sessions (intentional per comment `render.rs:165-167`) |
| 5.3 | Row radius 6 px | DONE | `render.rs:305,655,869,1219` |
| 5.4 | Hover `content(.05)`, selected `content(.10)` | PARTIAL | Project/session/node rows DONE (`render.rs:312-313,873-874,1225`). Group headers reuse Classic's header (`shell/project_rail.rs:1068-1081`: 32 px, 14 px text, opacity .65, open bg .05, hover .08, `rounded_md`) |
| 5.5 | Child rail: vertical line, curved into last child, reusing `rail_color` and existing bend | DONE | `render.rs:209` `rails::step(last, row)` → `activity::rail_branch_tinted` → `theme::rail_color()` |
| 5.6 | Theme functions only; sidebar/status bar `shell_fill()`, floating `floating_fill()`, status colors from theme with light variants | DONE | Status bar `render.rs:967`; peek panel `shell/sidebar_peek.rs:303-309` (`floating_fill` in Tree); lift plate `render.rs:99`; new `agent_running_color` / `agent_unread_color` with light variants (`theme.rs:329-336`). Hard-coded `rgb` only for group colour data (`render.rs:1084`) and shadow black |
| 5.7 | Acceptance in dark glass / dark opaque / light | PARTIAL | Test only asserts the collapsed bar is 48 px in each (`shell/layout.rs` test loop over `("dark",85),("dark",100),("light",85)`); the PRD's own note says desktop visual check not done |
| 5.8 | Badge edge colour follows the surface | PARTIAL | Sidebar rows blend over `shell_fill()` (`render.rs:768-774`), tabs over `title_tab_fill` (`shell.rs:9147-9153`). In the Tree peek panel the background is `floating_fill()` but the row badge edge still uses `shell_fill()` |

## 6. Status icon (scheme A + R1)

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 6.1 | Tile always provider style with logo, top highlight, 1 px edge | DONE | `session_mark.rs:195` `Face::provider`; `tab_tile.rs` |
| 6.2 | Corner badges: amber solid (needs you), hollow blue (background wait), cyan solid (unread done) | DONE | `session_mark.rs:196-201,243-248` |
| 6.3 | Badge 8 px tab / 7 px row; edge 2 / 1.5 px in surface colour | DONE | `session_mark.rs:191-194,204-205,236-238` |
| 6.4 | Running: light circling tile, 1.6 s, 24 % track, outside layout, concentric | DONE | `session_mark.rs:206-221` (1600 ms), `255-305` (track `opacity(0.24)`, canvas painted outside tile) |
| 6.5 | Geometry: tab 24×20 r5, gap 1, width 1.5, inner 6 / outer 7.5; row 16×14 r4, gap 1, width 1.25, inner 5 / outer 6.25 | DONE | `tab_tile.rs:42-51` (radius 5 / 4); `session_mark.rs:192-193,258-260` (`outset = 1 + width`, `outer = radius + outset`) |
| 6.6 | Drop tile shadow while running | DONE | `session_mark.rs:207` `face.flat()` |
| 6.7 | Reduce motion: ring static | DONE | `session_mark.rs:210-211` |
| 6.8 | Unread done: badge only, no extra green dot | DONE | `render.rs:733-753` trailing ✓ + time only |
| 6.9 | Tabs and tree rows share the component (Focused tabs change too) | DONE | `render.rs:775-782`; `shell.rs:8509-8519` |
| 6.10 | Worktree rollup dots amber / blue / cyan | DONE | `tree.rs:37-47` |

## 7. Row behaviour and archive

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 7.1 | Click any row enters the session, no preview overlay on click | DONE | `render.rs:909-913` → `session_groups.rs:510-511` |
| 7.2 | Archive via hover or right-click | DONE | hover `render.rs:821-825` (`session_archive_button`, `shell.rs:7360`); right-click `render.rs:930-945` |
| 7.3 | Auto-archive after branch merge or worktree cleanup | DONE (Tree only) | `progress_summary.rs:238-466`; `monocode-core/worktree_management.rs:4,10,57`; one-shot key `progress_summary.rs:361-367,454-460` |
| 7.4 | "Archived N" at bottom, not counted | DONE | see 3.12 |

## 8. Feature parity (Tree 布局的功能对齐)

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 8.1 | Project avatar = existing pixel mascots, manual or automatic, colour/logo kept, globe for remote | DONE | `render.rs:395-425` (`project_mark`, `globe`) |
| 8.2 | Avatar seed fix: automatic mascot from **folder name** everywhere | DONE | `monocode-core/project_appearance.rs:53-62` + test `automatic_mascots_use_the_folder_name_on_either_platform`; all callers go through `ProjectAppearance::mascot` (`project_settings.rs:103,299`, `conversation.rs:3415`) |
| 8.3 | Pinned projects area at top of tree | DONE | `tree.rs:634-637` |
| 8.4 | Groups as collapsible subheadings with colour and mascot | DONE (see follow-up e) | `render.rs:175-177` reuses `project_rail_group_header` |
| 8.5 | Drag reorder within region; groups draggable | DONE (see follow-up c) | `shell/project_rail.rs` `start_rail_drag` uses `tree.drag_blocks` filtered by `project_rail_peers`; tests `tree.rs:1574,1645,1742` |
| 8.6 | Move to group, Move up/down shortcuts, full project menu (new session, new worktree, pin, group, appearance, mute, editor, reveal, archive, delete) | DONE | `shell.rs:6526-6660` |
| 8.7 | Shares Classic data (`project_rail_pinned/groups/assignments`, `projects` order) | DONE | `tree.rs:629-662`; drop writes `workspace.projects` (`project_rail.rs` finish); `workspace.rs` test |
| 8.8 | Session features: pin, folders (new, drag in, drag onto session, colour, rename), reminders, rename, archive, delete, work item, copy ID, cloud actions, multi-select + batch, context menu, compact-row hover preview | DONE | Shared `MenuTarget::Session` (`render.rs:938-943`); rename `render.rs:689-691,476-485`; delete action `render.rs:29-34`; multi-select `session_groups.rs:470-520`, batch `session_menu_ids` `session_groups.rs:451-462`; folder-by-drop test `tree.rs:1718-1738`; hover preview `render.rs:902-908` (`compact` flag) |
| 8.9 | Filters: existing menu, ancestors of matches stay visible, no new search box | DONE | filter button in PROJECTS heading `render.rs:261-262`; `tree.rs:587-613` (root kept), `tree.rs:668-675,693-701,1020-1021` |
| 8.10 | Nav rows on top, visibility options from Focused | DONE | `render.rs:39-43` `sidebar_navigation`; `sidebar_list.rs:649-650` |
| 8.11 | Live agents card hidden | DONE | `tree_sidebar` never calls `live_agents_preview` (`render.rs:5-63`) |
| 8.12 | Focused top project switcher and bottom project strip removed | DONE | `shell.rs:7718-7720`; test asserts `sidebar-project-button` absent |
| 8.13 | Width drag, ⌘B collapse, edge slide-out kept | DONE | `shell/layout.rs:151-162`; `sidebar_peek.rs:66-69` (peek for non-Classic) |
| 8.14 | "Open folder on a machine…" reachable | DONE | PROJECTS "+" → `MenuTarget::ProjectRailAdd` (`render.rs:263-270`, `shell.rs:6343-6355`) |
| 8.15 | "New session in Codespace…" reachable | PARTIAL | The menu item exists only in `MenuTarget::Header` (Focused header, `shell.rs:6491-6500`), which Tree removes; the Tree project menu (`shell.rs:6526-6660`) has none. Only indirect path: composer cloud menu → `OpenExistingInstance` (`conversation/cloud_target.rs:836-840`, handled `shell.rs:1924-1937`), which needs a cloud backend already configured |
| 8.16 | Settings reachable (added, not in PRD) | CHANGED (addition) | Footer gear `render.rs:67-84`; status-bar gear `render.rs:1055-1064`; test `tree.rs:1244` |
| 8.17 | Project-row hover "+" for new session (added) | CHANGED (addition) | `render.rs:325-338`; test `tree.rs:1305` |

## 9. Collapsed state (48 px status bar) and peek

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 9.1 | 48 px bar, top-to-bottom: expand, nav icons with count/dot, amber Needs you N (click = Ctrl J), Mono avatars with status dot, project avatars | DONE (+ Settings gear at bottom) | `tree.rs:18`; `render.rs:949-1066`; nav `render.rs:1087-1159` |
| 9.2 | Same project order, Pinned and group colour bars | DONE | `render.rs:959,1015-1027` |
| 9.3 | Avatar corner dot = most urgent (needs you > running > unread); blue frame when running; hover shows counts | DONE | `render.rs:404-423` (`dot=true`), tooltip `render.rs:1029-1045` |
| 9.4 | Click avatar → tree slides out and scrolls to project; closes on session click or mouse leaving; edge hover also opens; pin or Ctrl+B keeps open | DONE | `render.rs:1046-1049`; `tree.rs:1166-1183,457-463`; `tree.rs:1117-1119`; pin `sidebar_peek.rs:333-344`. Mouse-leave close is inherited Focused peek behaviour (not re-tested for Tree) |

## 10. Shortcuts

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 10.1 | ⌘⇧↑/↓ step sessions in tree order | DONE | `shell.rs:7591-7594` → `tree.rs:1134-1144` |
| 10.2 | ⌘⇧←/→ previous/next project | DONE | `shell.rs:7662-7665` → `tree.rs:1146-1164` |
| 10.3 | ⌘1–8 still switch tabs | DONE | untouched |

## 11. Progress summary (2b)

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 11.1 | One-line summary per turn end with a quick model | PARTIAL | Trigger `shell/agent_activity.rs:222-235,340-344`; model = `background_text_model` (`progress_summary.rs:96`); prompt `progress_summary.rs:39-65`. Gaps: at most 4 concurrent jobs, a 5th turn end silently keeps the fallback (`progress_summary.rs:19,122-124`); remote chats never get a model summary (`progress_summary.rs:98` `!chat.is_remote()`); terminal sessions and failed/stopped turns get generic text (`progress_summary.rs:73-80`) |
| 11.2 | Shown as second line of unread done rows | DONE | `render.rs:723-732,880-900` |
| 11.3 | History restores latest turn's summary | DONE | `monocode-core/store/session_activity.rs:11,25`; `main.rs` restore; tests `store/session_activity_tests.rs` |
| 11.4 | Late/retried results never overwrite a newer turn; fallback when model unavailable | DONE | `progress_summary.rs:150-161` (job identity + `turn.at == ended_at`); fallback written first `agent_activity.rs:227-232` |
| 11.5 | Manual archive/restore from row menu | DONE | shared session menu |
| 11.6 | Same archive event once; restored session stays | DONE | `progress_summary.rs:361-367,454-460` |
| 11.7 | Evidence only affects matching ended sessions; drafts and running stay | DONE | `progress_summary.rs:208-236,351-358,392-412,437-451` |

## 12. Cross-child acceptance criteria (parent)

| # | Criterion | Status | Evidence |
|---|---|---|---|
| A1 | Dev server alone no longer "Working"; no stuck status | DONE (status-core; not re-audited in depth) | `monocode-core/provider/tasks.rs`, `agent_status.rs` |
| A2 | Same status in sidebar, project counts, queue, notifications, Mono plan | DONE for Tree surfaces | all read `Shell::agent_status` (`tree.rs:494`, `render.rs:699`); notifications/Mono not re-audited |
| A3 | No new personal settings | DONE | only the Layout option gained a value |
| A4 | Classic/Focused unchanged; Tree via settings, menu, shortcut | DONE | §1 |
| A5 | Mono avatars and animations visible in Tree | DONE | §2 |
| A6 | Every left-side feature of Classic/Focused has a Tree entry; pins/groups/order shared | PARTIAL | all except "New session in Codespace…" (8.15) |
| 2a-AC1 | Pinned / Reminders / folders / worktrees; archive at bottom | DONE | §3 |
| 2a-AC2 | Rows show question, step or unread summary; rollups from unified status | DONE | `render.rs:723-767` |
| 2a-AC3 | Expanded and collapsed share order; menus, selection, drag kept | DONE | `tree_project_entries` used by both (`tree.rs:629`, `render.rs:959`) |

## 13. Review follow-ups

| Item | Still present? | Evidence |
|---|---|---|
| (a) lingering unread demote replays after a layout switch | YES | `tree_settle_unread` is called only from `tree_sidebar` and `tree_status_bar` (`render.rs:9,952`). `set_navigation_layout` (`shell/layout.rs:181-197`) does not settle. Leave Tree with a lingering row, open another session in Focused/Classic, return to Tree → `tree.rs:831-841` inserts a fresh demote fade then |
| (b) opening an app view (Inbox/Today/Settings/Mono) does not demote the lingering row | YES | `tree.rs:823-830` only compares with `workspace.active`; app views leave `active` unchanged (row loses its selected fill via `app_view_open()` at `render.rs:693-694` but keeps the unread look) |
| (c1) header above scroll top excluded from drag | YES | `tree.rs:345-347` skips blocks whose header has no `bounds_for_item` |
| (c2) scrolling mid-drag leaves stale offsets | YES | bounds/viewport captured once at press (`project_rail.rs` `start_rail_drag`, `viewport: tree.then(...)`); scroll handler `tree.rs:265-271` → `shell.rs:6798-6806` does not update or cancel the drag; rows frozen while held (`render.rs:8`) |
| (c3) lift plate missing for one frame | NOT FIXED (unverified statically) | no code change targets it; plate is built from `tree_lift_bounds()` at `render.rs:17,88-108` before the list lays out |
| (d) project counts do not fade on collapse | YES | `counts_row(counts)` drawn unconditionally, no openness-based opacity (`render.rs:324,1231-1254`) |
| (e) group header expand/collapse does not animate | YES | group click toggles `g.collapsed` and persists (`project_rail.rs:1177-1187`); `tree_project_entries` just drops members (`tree.rs:640-651`); no `tree_disclose` for groups |
| (f) "Needs you 0" still amber | YES | `render.rs:601` `text_color(warning_color())` and `render.rs:618` `bg(warning_color().opacity(0.055))` unconditional; status bar `render.rs:989` |
| (g) heading styles inconsistent | YES | Tree headings: uppercase 10 px semibold `content(0.45)` (`render.rs:250-260`); Monos: `section_label("Monos")` title case 12 px medium `content(0.5)` with `px_2` + `pb_3` wrapper (`theme.rs:479-490`, `monos/mod.rs:1591,1606`); group headers Classic style (5.4) |

## Caveats / Not Found

- Nothing was built or run; all statuses come from reading code and existing tests.
- Mouse-leave close of the peek and the collapsed bar's Mono status dots are inherited from existing Focused/Classic code and were not traced line by line.
- "Worktree +" (3.6) has no source in the PRDs or artboard (`.ryugi/artifacts/artboard/agent-overview-sidebar/`); classified MISSING only because the audit request lists it.
