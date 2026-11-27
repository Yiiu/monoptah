# Research: R1 Tree nav strip + Sidebar options from Tree

- **Query**: `sidebar_navigation` and how Tree calls it; Sidebar options menu wiring; badges/counts; shortcuts; icon-button and tooltip helpers
- **Scope**: internal (origin/main 7106f9c1)
- **Date**: 2026-11-27

All paths under `crates/monocode-gpui/src/`, line numbers from `origin/main`.

## Findings

### Files Found

| File:line | What |
|---|---|
| `shell/project_tree/render.rs:12-86` | `tree_sidebar`: column = `sidebar_navigation(cx)` (:58), `tree_request_queue` (:59), virtual `gpui::list` (:60-83), `tree_footer` (:84) |
| `shell/sidebar_list.rs:647-903` | `sidebar_navigation` (shared by Tree and Focused) |
| `shell/sidebar_list.rs:292,491` | Focused builds `Row::Navigation` in its virtual list and renders it via `sidebar_navigation`. Classic does NOT use it (Classic uses `project_rail.rs:495` `original_rail_navigation`, own selectors at `project_rail.rs:828`) |
| `shell/sidebar_list.rs:576` | `const TRAILING_SLOT: f32 = 14.` |
| `shell/sidebar_list.rs:579-621` | `sidebar_options_menu_items()` |
| `shell/sidebar_list.rs:623-645` | `change_sidebar_option(action, cx)` (toggles workspace field, `remeasure`, `persist`, `cx.notify`; `sidebar-monos` also `apply_preferences` + `sync_settings`) |
| `shell.rs:446` | `MenuTarget::SidebarOptions` |
| `shell.rs:984 / 1502` | field `sidebar_options_anchor: AnchorSlot` / init |
| `shell.rs:6128-6141` | menu subscription: `Picked` on SidebarOptions calls `change_sidebar_option` then `menu.replace_items(sidebar_options_menu_items())` so the menu stays open |
| `shell.rs:6344-6349` | `menu_items`: SidebarOptions -> ("Sidebar options", items, width 184., `MenuStyle::Filter`) |
| `shell.rs:6694` | `run_menu`: SidebarOptions -> `change_sidebar_option` |
| `shell.rs:10513-10515` | popover: SidebarOptions -> `.align(Align::Start)` |
| `shell.rs:7769-7793` | `Shell::sidebar`: Classic -> `classic_sidebar`, Tree -> `tree_sidebar` (early return), else Focused body |
| `shell.rs:7924-7950` | Focused's only opener: `icon_button("sidebar-options","Sidebar options","more-horizontal")`, `.debug_selector("sidebar-options")`, `.aria_expanded(menu_open(&SidebarOptions))`, `.relative().child(self.sidebar_options_anchor.probe())`, on_click toggles `open_menu(MenuTarget::SidebarOptions, Anchor::Slot(sidebar_options_anchor.clone()), ..)` / `close_menu` |
| `theme.rs:585-638` | `icon_button(id: &'static str, label: &'static str, path)` -> 24px `Stateful<Div>` (role Button, aria_label, group "chrome-button", hover fill `content(0.1)`); `icon_button_hover`, `icon_button_enabled` |
| `cloud_mark.rs:98-113` | `pub(crate) struct CloudTip(pub SharedString)` tooltip view (max_w 320, `shell_fill`, border `content(0.12)`, 12px) — already used by `shell_task_chip` |
| `shell/project_rail.rs:307-321` | `pub(super) struct ProjectTooltip(pub(super) String)` tooltip view (`pane_fill`, border `content(0.15)`) — used at `project_rail.rs:1548`, `shell.rs:7551` |

### Current `sidebar_navigation` behavior (sidebar_list.rs:647-903)

- `tree = workspace.tree_layout()`; `search_field = sidebar_search_visible && (classic_layout || tree)` (:650-651); `focused = !classic_layout` (:652) — note `focused` is also true in Tree.
- Peek pin: `(tree && self.peek_open()).then(icon_button("tree-peek-pin", "Keep sidebar open", "pin"))` (:654-660). Placed at the end of the Search row (:754) or, when Search is hidden, in its own 32px row justified end (:757-764).
- Container: `px_2`, `pt 4 / pb 12` for non-classic (:661-676), `flex_col gap_px`.
- Search row (:677-756): 32px; Tree look `px_2 bg(content(0.04))` (:698); hint text `format!("{MOD}K")` in Tree (:747-751), selectors `sidebar-search-field`, `sidebar-search-hint`; click = `focus_search`.
- Item list (:765-901): fixed array `[("Today","clock"),("New","square-plus"),("Inbox","inbox"),("Notes","note"),("Automations","zap"),("Contributors","chart-square")]`.
  - Filter (:775-786): `New` => `focused` (so New is shown in Tree too); Today => `sidebar_today_visible`; Notes => `preferences.notes && sidebar_notes_visible`; Automations => `sidebar_automations_visible`; Contributors => `sidebar_contributors_visible`; Inbox => `sidebar_inbox_visible`.
  - Each row is a 32px full-width row (`h(px(32.))`, icon 16 + label 14px MEDIUM) => 6 rows x 33px ≈ 198px of the Tree header.
  - Selected: New => `blank_session` (`active_session_is_blank`, :905-913); others => `self.app_view_label() == Some(label)` (:793-797).
  - Counts:
    - Today: `self.today.as_ref().map_or(0, |t| t.read(cx).open_count())` (:798-802; `today.rs:462` returns cached `open_count`).
    - Inbox: unread dot when `inbox.read(cx).unread_count() > 0` (:803-807; `inbox.rs:436`).
    - Notes: `self.note_count.map(|count| count.to_string())` (:849-851). `note_count: Option<usize>` (`shell.rs:918`), filled by `load_note_count` (`shell/app_views.rs:135-152`, background `list_notes().len()`, then `notify_sidebar`) and by observing the Notes view (`app_views.rs:283-290`). It shows the number even when it is `Some(0)`.
  - Trailing column (:843-882): New => `"{MOD}T"`; Today => count when > 0; Notes => count; Inbox => 6px accent dot (8px in Classic). Selector `sidebar-navigation-trailing-{label}`; Tree uses `min_w(TRAILING_SLOT).justify_end()`.
  - aria_label: "Inbox, new items" / "{label}, {n} open" / label (:812-818).
  - Click (:883-891): New `new_chat`, Today `open_today`, Inbox `open_inbox`, Notes `open_notes`, Automations `open_automations`, Contributors `open_contributors(None, None, ..)`.
  - Hover (:892-899): writes `self.sidebar_hover: Option<&'static str>` (`shell.rs:947`) then `notify_navigation` (:914-920; non-classic notifies `sidebar_frame`).
- `MOD` = `"⌘"` on macOS else `"Ctrl+"` (`shell.rs:513-517`).

### Keyboard shortcuts per item

- Only two are bound: New = "Tab: New" `⌘T` (`settings/pages/keybindings.rs:124-129`), Search = "App: Search" (`keybindings.rs:22`). Today, Inbox, Notes, Automations, Contributors have **no** binding in `BINDINGS` (`keybindings.rs:14-357`).
- `crate::settings::command_shortcut(&prefs, command) -> Option<String>` (`settings/pages/keybindings.rs:368-381`) returns the user-overridden or default label (non-mac replaces `⌘` with `Ctrl+`). Example use: `shell.rs:6610-6612`, `shell.rs:8034-8038`. The existing nav rows hardcode `{MOD}T` / `{MOD}K` instead.

### Sidebar options menu contents (sidebar_list.rs:579-621)

Toggles (ids): `sidebar-search`, `sidebar-today`, `sidebar-notes` (disabled unless `preferences.notes`), `sidebar-automations`, `sidebar-inbox`, `sidebar-contributors`, `sidebar-monos` (= `preferences.monos`), `sidebar-activity` (disabled unless `preferences.working_agents`), separator, `compact-session-cards`.
- There is no toggle for "New" (always shown in non-classic).
- `sidebar_activity_visible` drives `live_agents_preview` in the Focused body (`shell.rs:7795,7953`); `tree_sidebar` does not render live agents. `compact_session_cards` affects Focused cards (`shell.rs:7001`). Both have no visible effect in Tree today.
- The workspace fields are the same ones Focused uses (persisted by `persist`), so state is shared across layouts already.

### Smallest seam for opening the menu from Tree

Everything except a trigger already exists: Tree only needs an element that calls `open_menu(MenuTarget::SidebarOptions, Anchor::Slot(self.sidebar_options_anchor.clone()), window, cx)` (toggle with `close_menu` when `menu_open(&MenuTarget::SidebarOptions)`) and renders `self.sidebar_options_anchor.probe()` inside itself — copy of `shell.rs:7924-7950`. Only one of Focused/Tree renders at a time (`shell.rs:7791-7793` early return), so reusing the same anchor slot and the `sidebar-options` debug selector does not collide. Popover alignment (`shell.rs:10513`) and keep-open-on-pick (`shell.rs:6128-6141`) apply automatically.

### Tooltip pattern

`.tooltip(move |_, cx| cx.new(|_| CloudTip(text.clone())).into())` on a `Stateful<Div>` (example `shell/rails.rs:823-826`). `icon_button` returns `Stateful<Div>` so `.tooltip(..)` can be chained. No shared "label + shortcut" tooltip component found; `CloudTip` / `ProjectTooltip` render a single string.

### Tests pinning current behavior (need updating if Tree rows change)

| Test | File:line | Pins |
|---|---|---|
| `tree_sidebar_keeps_the_content_gutter_while_closing` | `shell/layout.rs:977-1032` | In Tree, `sidebar-search-field`/`sidebar-search-hint` and `sidebar-navigation-New`/`sidebar-navigation-trailing-New` right edges differ by 8px |
| `tree_shortcuts_hide_the_sidebar_and_allow_peek` | `shell/layout.rs:1034-1110+` | In Tree, `sidebar-navigation-Today` and `-Inbox` exist and keep the same width docked vs Peek; `tree-peek-pin` beside Search (same center y, `search.right() < pin.left()`); with Search hidden, Today top >= pin bottom. Also named in `.trellis/spec/native/component-guidelines.md` |
| `focused_navigation_trailing_items_share_the_filter_axis` etc. | `shell/sidebar_list.rs:983-1068` | Focused layout (default `NavigationLayout::Focused`, `workspace.rs:73-78`) — unaffected if change is gated on `tree` |
| `sidebar_visibility_switches_stay_open_preserve_notes_and_survive_restart` | `shell/sidebar_list.rs:1159-1291` | Focused: `sidebar-options` button, menu width 184 left-aligned, `menu-action-sidebar-*` rows, toggling hides `sidebar-navigation-Notes/Automations/Inbox`, `sidebar-search-button`, `new-mono`; persisted via `Workspace::restore` |
| `sidebar_hover_updates_labels_without_rebuilding_the_chat` | `shell.rs:13289-13312` | Focused (default): hovering `sidebar-navigation-Today/Inbox/Notes` sets `sidebar_hover` |
| contributors `LAYOUTS` tests | `shell/contributors_tests.rs:44-150` | "new layout"/"full rail"/"compact rail" (Focused + Classic) `sidebar-navigation-Contributors` / `rail-contributors` |
| `tree_layout_reaches_settings_open_in_peek_and_by_shortcut` | `shell/project_tree.rs:1342-1399` | `tree-settings` footer button (spec-required entry point) |

## Caveats / Not Found

- No keybinding exists for Today/Inbox/Notes/Automations/Contributors, so their tooltip can only show a name.
- `note_count` is `Some(0)` when the store has no notes; today's Notes trailing shows "0".
- Linux portability: tests must use `secondary-` keystrokes; `MOD`/`command_shortcut` differ per OS, so asserting exact shortcut text in a tooltip is platform-dependent.
