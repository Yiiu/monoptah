# Research: UI rules and reusable components for the Agent CLIs redesign

- **Query**: Design rules, reusable component inventory mapped to the approved Ryugi design, current Providers page / CLI details dialog, settings search + layout tests, light/dark handling for permission colours.
- **Scope**: internal (repo) + design artboard
- **Date**: 2026-11-24

All paths are relative to `crates/monocode-gpui/src/` unless they start with `.trellis/`, `.ryugi/`, `docs/` or `crates/`.

---

## 1. Design rules that apply

### 1.1 Sources

| Source | What it says |
|---|---|
| `CLAUDE.md` "Developing the native app" | "Fix at the root, in the shared component": prefer `modal.rs`, `popover.rs`; a fix in individual callers reached 2 of ~35 dialogs. Tests Linux-portable, `secondary-` keystrokes. |
| `.trellis/spec/native/component-guidelines.md` | The normative native UI rules (summarised below). |
| `.trellis/spec/native/directory-structure.md:57-58` | "A settings row on an existing page → `src/settings/pages/<page>.rs`, using `self.row`/`toggle`/`segmented`/`select` from `settings/widgets.rs`; field on `Preferences`, changed via `SettingsView::update`." |
| `.trellis/spec/guides/code-reuse-thinking-guide.md:147-163` | Shared-pieces table: dialogs `modal.rs`, menus/dropdowns/choice selects `popover.rs`, colours `theme.rs`. Non-drawing logic (parsing, storage) goes to `monocode-core`. |
| `.trellis/spec/native/state-and-async.md` | Preference flow (`SettingsView::update` → `SettingsEvent::Changed` → `shell/settings.rs` persist), background work shape, user-facing errors are English strings stored on the entity. |
| `.trellis/spec/native/testing.md` | `debug_selector` + `visual.debug_bounds`, `set_reduce_motion(true)` for animated surfaces, run only touched modules. |
| `docs/native/*` | No visual-design doc for the app UI. `docs/native/visuals.md` is about agent-published inline visuals (not app chrome); not applicable. |

Key rules from `component-guidelines.md`:
- Stateless building blocks are plain fns returning `Div`/`Stateful<Div>` or `RenderOnce` builders; prefer these over new entities for pure presentation.
- Every interactive element: `.id(..)`, a `Role`, an `aria_label` (English). Elements tests find get `.debug_selector(..)`.
- Colours only through theme functions, never hard-coded hex in views: `content(alpha)`, `accent_color()`, `pane_fill()`, `shell_fill()`, `floating_fill()`, `danger_color()`, `warning_color()`, `success_color()`, `is_light()`.
- "Interaction fills are translucent ink (`content(0.07)` on hover), not opaque greys" (also `theme.rs:2-3`).
- "Settings pages use their own `select`/`segmented`/`toggle`/`slider` helpers in `settings/widgets.rs`; use those inside Settings."
- Icons: `icon(name, side)`; unknown names render nothing; new icons are added in `assets.rs` (explicit `include_bytes!` arm or stroke body in `icon_body`).
- Do not hand-roll a modal card (`modal::card()` / `modal::surface`).

### 1.2 gpui-kit / gpui-component skills

`crates/monocode-gpui/Cargo.toml:15-16,104-108` depends only on vendored `gpui-pre` 0.3.9 (`vendor/gpui-pre*`). There is no `gpui-component` / `gpui-kit` dependency anywhere outside vendor (grep of all `Cargo.toml`). The `gpui-kit-design-guides` and `gpui-component` skills therefore do **not** apply as normative rules; this app has its own hand-built component layer (`theme.rs`, `modal.rs`, `popover.rs`, `settings/widgets.rs`) ported from the React app's Tailwind metrics.

### 1.3 Theme tokens (`theme.rs`)

| Token | Location | Notes |
|---|---|---|
| `CHAT 0x141414`, `TEXT 0xebebeb`, `MUTED 0x969696`, `ACCENT 0x4da3f5` | `theme.rs:4-7` | Fallbacks before `configure_project` runs. |
| `content(alpha)` | `theme.rs:226-236` | Ink at an opacity; ink comes from the workspace palette (`settings/palettes.rs`, 11 palettes, light + dark each). |
| `accent_color()` | `theme.rs:218-225` | Palette accent or user custom accent. `picked_accent()` `:261` = user accent if set. |
| `is_light()` | `theme.rs:322-324` | Theme switch for colour choices. |
| `warning_color()` amber `0xb45309`/`0xfbbf24` | `theme.rs:326-328` | light/dark |
| `success_color()` green `0x059669`/`0x34d399` | `theme.rs:330-332` | light/dark |
| `danger_color()` red `0xdc2626`/`0xf87171` | `theme.rs:334-337` | light/dark |
| `floating_fill()` | `theme.rs:339-350` | Opaque popover/modal card fill. |
| `shell_fill()`, `pane_fill()`, `background_base()`, `inset_pane_fill()` | `theme.rs:379-428` | Surfaces. |
| `section_label(label)` | `theme.rs:430-442` | 28px, 12px medium, `content(0.5)`. |
| `icon(path, side)` | `theme.rs:443-450` | Tinted `content(0.5)`. |
| `provider_icon(provider, side)` | `theme.rs:502-534` | Provider logo (monochrome ones follow ink). |
| `provider_tile` / `provider_glyph` | `theme.rs:452-500` | Brand-coloured tile + white glyph (used by tab tiles). |
| `icon_button(id,label,path)` / `icon_button_enabled` / `icon_button_hover` | `theme.rs:536-592` | 24px chrome button, 14px icon, hover `content(0.1)`. |
| `FOOTER_TEXT_SIZE 11`, `FOOTER_ICON_SIZE 12` | `theme.rs:630-632` | |
| `white(a)`, `black(a)` | `theme.rs:653-659` | |
| Mono font | `crate::fonts::mono()` (`fonts.rs:32`) | Used e.g. `settings/management.rs:512`, `:1191`. |

There are no radius constants in `theme.rs`; views use GPUI's `rounded_md` (6px), `rounded_lg` (8px), `rounded_xl` (12px), and `modal::RADIUS = 16` (`modal.rs:14`), `modal::BORDER = 0.1` (`modal.rs:21`).

Settings-local colour helpers in `settings/widgets.rs:13-26`: `RED = 0xf87171` + `red(alpha)` (fixed, not light-aware), `accent(alpha)` (accent at alpha).

Text sizes used in Settings (`settings/widgets.rs`): page title 20/25 semibold; group title 13/19.5 semibold; row label 13/19.5 medium; descriptions 12/19.5 `content(0.45)`; controls 12/16; `note()` 12/16 `content(0.45)`.

---

## 2. Component inventory mapped to the design

Design source: `.ryugi/artifacts/artboard/agent-clis-settings/pages/design/claude.html` (all design pages share the same CSS lines 4-171 and render script lines 174-326; pages differ only in `window.__STATE`). Tokens: `pages/tokens.css`.

### 2.1 Existing settings building blocks (`settings/widgets.rs`)

| Fn | Line | Shape |
|---|---|---|
| `page_header(title, description)` | 29 | |
| `note(text)` | 53 | muted line inside a group card |
| `secondary_button(id, label, danger, disabled)` | 65 | bordered `content(0.1)`, `rounded_md`, px 10 py 4, 12px; caller adds children (icon + text) |
| `checkbox(id, label, checked, disabled)` | 92 | 14px |
| `icon_action(id, label, path, side, icon_side, alpha, disabled)` | 152 | borderless square icon button |
| `primary_button(id, label, disabled)` | 187 | solid `content(1.)` fill, ink = shell fill |
| `SettingsView::group(id, first, title, description, action, children)` | 215 | title + optional right action, card `rounded_xl`, border `content(0.1)` (accent 0.6 when flashing), bg `content(0.03)` |
| `SettingsView::row(id, first, label, description, controls)` | 279 | px 16, py 14, top rule `content(0.05)`, controls right (max 60% width), stacks under 560px unless id is in the hard-coded `toggle_only` list (`:290-322`); emits `settings-row-{id}` selector and `reveal_probe` |
| `reveal_probe(id)` | 392 | search landing target |
| `SettingsView::toggle(id: &'static str, label, on, disabled, cx, change)` | 404 | 36x20 switch, 16px knob, accent when on, selector `settings-toggle-{id}` |
| `SettingsView::segmented(id, label, value, options, cx, change)` | 447 | hairline frame, selected `content(0.1)`; selector `settings-{id}-{option}`; `Role::RadioGroup`/`RadioButton` |
| `SettingsView::slider(..)` | 507 | |
| `SettingsView::select(id: &'static str, label, value, options: Vec<(String, SharedString, Option<AnyElement>)>, width, cx, change)` | 597 | bordered trigger + `popover()` listbox with icons, check on selected, keyboard, filter field when > 8 options (`SEARCHABLE_SELECT`, `:886`); one open at a time via `self.select_open: Option<&'static str>` |
| `refresh_button(id, label, cx)` | 917 | secondary button + `refresh-cw` icon, disabled while `discovery_loading` |

Note: `toggle` and `select` require `&'static str` ids (and `row`/`group`/`reveal_probe` ids are `Option<&'static str>`). `Kind::id()` returns `&'static str`, so per-agent ids can be `kind.id()` itself; composed ids (`format!`) are not `'static`.

### 2.2 Shared layer outside Settings

- `modal.rs`: `card()` `:132`, `card_body` `:148`, `title` `:151`, `body` `:159`, `actions` `:167`, `ghost_button` `:194`, `danger_ghost_button` `:203`, `primary_button` `:212`, `danger_button` `:225`, `check_row` `:312`, `radio_row(id, selected, disabled, label, notes)` `:323` (14px radio marker + label + notes, via `option_row` `:247-308`), `disabled(button, bool)` `:333`, `overlay` `:346`, `window_overlay` `:80`, `scrim` `:48`, `surface(radius)` `:102`.
- `popover.rs`: `popover(id, Anchor)` `:333` builder (`.side`, `.align`, `.width`, `.max_height`, `.padding`, `.on_dismiss`, `.modal_surface`); `AnchorSlot` `:208` (`probe()`, `layout_probe()`); `Menu` `:1061` + `menu_popover(&menu, anchor, cx)` `:1914`; `MenuAction` `:869` builder with `.icon`, `.icon_color`, `.description`, `.shortcut`, `.value`, `.badge`, `.checked`, `.toggle`, `.danger`, `.disabled`, `.submenu`, `.flyout`; `MenuItem::{Header, Separator, Action, Note, Custom}` `:973`; `MenuStyle` `:813` incl. **`Access`** ("AccessPicker: medium title, wrapping hint, selected row filled instead of checked", icon 14px at `content(0.7)` unless `icon_color`; `:1356-1400`, `:1619-1628`); `Choices<T>` `:1959` single-choice dropdown (`open`, `close`, `popover`).
- Existing permission picker precedent: composer access menu `conversation.rs:3148-3151` (`MenuStyle::Access`, width 288) built from `ACCESS_MODES` `conversation.rs:94-119` and `conversation/mono.rs:514-543` (`mono_access_options`: icon + description per mode, full access tinted `AMBER_400`). Composer trigger icon per mode `conversation.rs:3600-3616` (`square-lock`, `pencil-edit`, `sparkles`, `shield-alert`). Core enum `monocode_core::agent::AccessMode` (`crates/monocode-core/src/agent.rs:7-42`: ReadOnly, Supervised, AutoAcceptEdits, Auto, FullAccess).
- Notifications: `notifications/surface.rs` `glass_frame()` `:27`, `toast_frame()` `:42` (top-right stack card); `notifications/errors.rs` `card` / `severity_card(id, Severity::{Error,Warning}, message, dismiss)` `:13-..` (rose/amber tinted, `circle-alert`/`triangle-alert`); `notifications/layout_notice.rs` (status card with actions); `notifications/harness_updates.rs` (per-agent "update available"/updating/updated/failed rows — the existing update flow).
- Spinner: `right_panel::spinner(id, path, side)` `right_panel.rs:166` (1s looping rotate via `pulse::looping`). Text shimmer: `activity::shimmer` `activity.rs:11`. Ad-hoc rotations: `accounts/ui.rs:105`, `notifications/updater.rs:348`.
- Tooltip: no shared tooltip widget. Local hover-card views: `cloud_mark::CloudTip` `cloud_mark.rs:98-113` (`.tooltip(|_, cx| cx.new(|_| CloudTip(..)).into())`, max-w 320, `shell_fill`, border `content(0.12)`, 12px), plus `conversation/cloud_panel.rs:19`, `contributors/ui.rs:120`, `monos/workflows_ui/graph/cards.rs:931`, `shell/contributors.rs:113`.
- Text input: `input::Input` entity (`input.rs`). Constructors `Input::field(label, placeholder, placeholder_color, cx)` `input.rs:371`, `Input::multiline_field(label, placeholder, cx)` `:414` (fixed 124px), `commit_message` (grows 20→152). Setters: `set_font_size` `:565`, `set_text` `:642`, `set_read_only` `:559`, `set_secret` `:293`, `set_placeholder` `:545`, `set_min_height`/`set_max_height`/`set_line_height` `:571-580`. Events: `Changed`, `Submitted`, `AlternateSubmitted`, `Rejected` (`:281-285`). No blur event on `Input`; blur-commit is done with `cx.on_blur(&focus, window, ..)` (e.g. `contributors/identities/view.rs:311`, `contributors/ticket/draw.rs:318`, `explorer.rs:317`; tests `contributors/identities_tests.rs:1027` "commits_a_rename_on_blur").
- Input chrome in Settings is built per page (no shared settings text-field wrapper): `management.rs:1173 form_field` (label + box `rounded_md`, border `content(0.1)`, bg `content(0.04)`, mono font for JSON), `pages/connections.rs:189-217 connection_field` (36px, `rounded_lg`, border `content(0.15)`), `pages/mobile.rs` remote field.
- Copy feedback: `inbox/ui.rs:1189-1197` animated check after copy (`COPIED`); clipboard via `cx.write_to_clipboard(ClipboardItem::new_string(..))` (`settings/pages/mobile.rs:298`, `skills.rs:494`, `web_app.rs:561`). Reveal in file manager: `cx.reveal_path(path)` (`management.rs:959-963`).
- Shell splitting: `monocode-core` already depends on `shlex = "2"` (`crates/monocode-core/Cargo.toml:14`).

### 2.3 Expandable row precedents

- `settings/notification_preferences.rs:796-866`: project row = button with icon, name (13 medium) + detail (12, 0.45), `chevron-right`/`chevron-down` icon swap, one expanded at a time via `notification_state.expanded: Option<String>`, expanded body below with `border_t_1()` `content(0.05)` + `px_4()`.
- `settings/skill_document.rs:123` chevron-right; `settings/pages/mobile.rs:775-797` disclosure text toggle with `aria_expanded`.
- `widgets.rs:687-694` rotates `chevron-down-hi` 180° with `with_transformation(Transformation::rotate(..))` when open (rotation pattern for a chevron).

### 2.4 Mapping table (design element → reuse or new)

| Design element (claude.html) | Verdict | Where |
|---|---|---|
| Section title "Agent CLIs" + description | REUSE | `SettingsView::group` (existing call `providers.rs:401-408`) |
| Sub-group header "Installed (N) · Checked … · Refresh" / "Available to install (N)" (`.gh`, `.count` mono 11px) | REUSE + small NEW | `group(..)` title + `action` slot with `refresh_button` (`widgets.rs:917`). Mono count text: NEW inline text using `fonts::mono()`. |
| Settings card `.card` (r 10, border ink .09, bg raised) | REUSE | `group` card (`rounded_xl`, `content(0.1)`, `content(0.03)`) |
| Simple row `.srow` (label + description + control) | REUSE | `SettingsView::row` |
| "Open new sessions in" 3 radio cards with previews (`.modes`/`.mode`, selected border accent + bg accent .06, radio dot) | NEW | No radio-card widget exists. Closest pieces: `modal::radio_row` marker (`modal.rs:247-308`) and `segmented` (`widgets.rs:447`, `Role::RadioGroup`). Build as a new settings widget alongside `segmented`. |
| "Opens in" Chat / Terminal (TUI) cards in expanded panel (`.ocards`/`.oc`) | NEW (same radio-card widget, compact variant) | as above |
| Row Chat/TUI small segmented with icons (`.seg.sm`) | REUSE (+option) | `segmented` (`widgets.rs:447`) — currently text-only (comment at `providers.rs:224-225`: "The icons of the web app's control have no native widget here, so both options are text"). Icons would need an extension of `segmented`. |
| Enable switch (`.sw` 32x18) | REUSE | `SettingsView::toggle` (36x20) |
| Permission picker trigger (`.pbtn`: tinted 24px icon tile + label + `chevrons-up-down`, red border for full) | NEW trigger, REUSE list | Trigger: extend `select` trigger look (`widgets.rs:642-694`) or new. List: `Menu` with `MenuStyle::Access` (icon + title + description, selected filled) via `menu_popover`, or `select` options with icon elements. `MenuAction::icon_color` exists for per-level tint. |
| Permission option code line (`<code>--permission-mode …</code>`) | NEW (or reuse `MenuAction::description`) | `MenuAction` has one `description`; second mono line not supported. |
| "Same as all agents" inherit option + separator + footnote | REUSE | `MenuItem::Separator`, `MenuItem::Note` (`popover.rs:973-989`) |
| Permission badge `.pbadge` (tinted bg .13, icon 12 + text 11 medium) | NEW | No shared badge/tag fn. Precedents: `providers.rs:146-160` INSTALLED pill (`rounded_full`, `content(0.1)`, 10px); `contributors/leaderboard.rs:251 badge_pill`; `questions.rs:535 badge`; `MenuAction::badge`. |
| Tags "Default" (accent tint), "Off", "TUI" (`.tag`) | NEW (same badge widget) | as above |
| Warning box under picker for Full access (`.warn`) | NEW (inline) | Colour from `danger_color()`; error-card precedent `notifications/errors.rs:severity_card` (toast-sized, not inline). |
| Provider avatar `.av` 28px coloured tile | REUSE | `provider_icon(kind.id(), side)` (current page uses 16px, `providers.rs:117`) or `provider_tile`/`provider_glyph` (`theme.rs:452-500`) |
| Collapsed row header (chevron, avatar, name+tags, mono summary, version/update, switch, ellipsis) | NEW row widget | Pattern from `notification_preferences.rs:796-866`; chevron icon `chevron-right` + rotate (`widgets.rs:687-694`). |
| Mono summary line (11px mono, `content(0.45)`, truncate) | NEW inline | `fonts::mono()` |
| Version text `.ver` / Update button `.upd` (accent bordered, `arrow-up`) | REUSE + NEW | Update logic: `notifications/harness_updates.rs`. Button: `secondary_button` with accent tint (new variant) |
| Ellipsis row menu (`.menu`: Use by default / Clear default / Check version / Open CLI location / Copy launch command / Reset all / Docs) | REUSE | `Menu` + `MenuAction::icon` + `MenuItem::Separator` + `menu_popover` (`popover.rs:1914`); `MenuAction::danger()` exists. Trigger: `icon_action` (`widgets.rs:152`) or `icon_button` (`theme.rs:536`) with `more-horizontal` icon. |
| Expanded panel field grid (`.f`: 136px label column + control column, top rules) | NEW | No two-column field layout widget; `management.rs:1173 form_field` and `labeled_control` `:1198` are stacked label-over-control. |
| Text input single line mono (`.inp.mono` 28px, border ink .12) | REUSE entity, NEW chrome | `Input::field` + `set_font_size(12.)`; wrapper box per `management.rs:1173-1196`; mono via `font_family(fonts::mono())` on the wrapper (as `management.rs:1190-1192`). |
| Multi-line env editor (`textarea.mono`) | REUSE | `Input::multiline_field` (`input.rs:414`) or `commit_message`-style growing height |
| Field-level error text `.help.e` (`circle-x` + red) | NEW inline | Existing: `management.rs:~925-935` `Role::Alert`, 11px, `rgb(0xf87171)`; colour should be `danger_color()` |
| "Saved" flash (`check` + green) | NEW | No existing "Saved" inline flash in Settings; `success_color()`; timer pattern like `reveal_setting` (`settings/mod.rs:710-724`, 1.8s) or `inbox/ui.rs` COPIED pop |
| Status line with dot (green auto-detected / amber configured, path, version) | NEW inline | Content today in `management.rs:466-528` (Global path · Auto-detected / Configured / Restart required / Needs attention; path + version mono 10px) |
| Model select with "Agent default" + "Custom model…" | REUSE | `SettingsView::select` (supports icons, search when > 8) + `Input::field` for custom |
| Effort ladder (`.lad`: buttons with bar glyph + label, pressed state) | NEW | Extend/sibling of `segmented`; bar glyph is new drawing |
| Arguments chips (`.chip` mono 11, ink .08) | NEW | same family as badge widget |
| Environment table (`.envt` rows key/value/eye) | NEW | Masking: `Input::set_secret` exists for inputs; read view needs new rows. Eye button: `icon_action`. |
| Reveal/hide secret (`eye`/`eye-off`) | REUSE button, NEW icon | `icon_action`; `eye-off` missing (see icons) |
| "Edit as text" / "Add variables" small ghost buttons | REUSE | `secondary_button(..).border_0()` (pattern at `management.rs:1131-1136`) |
| Launch preview box (`.prev`: header + mono pre + Copy) | NEW | Mono block precedent `management.rs:502-531` (border `content(0.1)`, bg `content(0.03)`, mono) |
| Copy button + copied feedback | REUSE pattern | `cx.write_to_clipboard`; `inbox/ui.rs:1189-1197` |
| Install row for undetected agents (hint code + Copy + Install accent button) | NEW layout, REUSE buttons | `secondary_button`, `primary_button` |
| "Browse…" button | REUSE | `secondary_button` + `folder-open` |
| Toast (bottom-right, icon + html + Undo) | REUSE frame / NEW host | `notifications/surface.rs toast_frame` / `glass_frame` + `notifications/errors.rs`. No generic in-Settings toast queue exists; current notices are shell-level top-right stack. |
| Detection skeleton rows (`.skel`) + "Detecting installed agents…" with spinner | NEW skeleton, REUSE spinner | `right_panel::spinner(id, "loader", 14.)`; today only `note("Checking installed CLIs…")` (`providers.rs:313-315`) |
| Detection error banner (`.banner` red tint + Retry) | NEW inline | colours `danger_color()`; button `secondary_button` |
| Tooltip on Refresh / switch (`title=`) | REUSE pattern | GPUI `.tooltip(..)` with a `CloudTip`-like view (`cloud_mark.rs:98`); no shared settings tooltip |

### 2.5 Icons (design name → asset)

`assets.rs` explicit arms `:6-196`, stroke bodies `icon_body` `:202-268`.

| Design icon | Present? | Existing name / nearest |
|---|---|---|
| settings-2 | no | `settings` (`assets.rs:96`), `sliders` (`:116`) |
| hand | no | — |
| pen-line | no | `pencil-line` (`icon_body`), `pencil-edit` (`:110`), `pencil` (`icon_body`) |
| shield-check | no | `shield` (`icon_body`), `shield-alert` (`:115`) |
| shield-off | no | `shield-alert` |
| message-square | yes | `:173` |
| terminal | yes | `:146` |
| copy | yes | `icon_body` |
| folder-open | yes | `:106` (also `open-folder`) |
| refresh-cw | yes | `:136` |
| eye | yes | `:137` |
| eye-off | no | — |
| lock | yes | `icon_body`; also `square-lock` `:118` |
| chevron-right | yes | `:73` (and `icon_body`) |
| chevrons-up-down | yes | `:70` |
| ellipsis | no | `more-horizontal` `:134` |
| download | no | `arrow-down-circle` `:183`, `arrow-down` (`icon_body`) |
| arrow-up | yes | `icon_body` |
| check | yes | `:101` (`check-hi` `:185`) |
| star | yes | `:119`, `star-filled` `:120` |
| star-off | no | — |
| link | no | — |
| rotate-ccw | yes | `:180` |
| book-open | no | — |
| circle-x | yes | `:38` |
| minus | yes | `icon_body` (`minus-sign` `:153`) |
| plus | yes | `:52` |
| square-terminal | no | `terminal` |
| loader | yes | `:135` (`loader-03` `:39`) |
| info | no | `circle-alert`, `alert-circle` `:40`, `circle-help` |
| layout-list | no | `list-bullet` `:151` |
| arrow-right-left | no | — |
| x | yes | `:145` |

Assets come in two families: Hugeicons files under `crates/monocode-gpui/assets/hugeicons/` (1.75 stroke, comment at `assets.rs:98-99`) and Lucide-style stroke bodies in `icon_body` (1.6 stroke, wrapped at `assets.rs:194`).

---

## 3. Current Providers page and CLI details

### 3.1 `settings/pages/providers.rs` (417 lines) — `providers_page`

Returns five groups (`:385-416`):
1. **Accounts** `provider-accounts` (`:386-393`) — from `crate::accounts` `settings_rows`. Unchanged per PRD.
2. **Usage and privacy** (`:394`) — rows `show-remaining-usage`, `mask-emails` (`:27-56`). Unchanged.
3. **Agent CLIs** `agent-clis` (`:395-402`) — the target. Group action = **scope select** `provider-scope` (Global / per project, `:57-82`). For each `Kind::ALL` (`:85-311`):
   - label: `provider_icon` 16px, name, `folder-open` button opening CLI details (`:121-145`, uses `binary_anchors` slot), INSTALLED / NOT INSTALLED pill (`:146-160`);
   - controls: model `select` (`:196-222`, from `self.ctx.catalogs`), "Use by default" / "Default" secondary button (`:162-194`, disabled when no model or not installed — the bug the PRD fixes), session-surface `segmented` Chat / Terminal (TUI) (`:226-248`, hidden with `NO_TERMINAL_HINT` when `ctx.no_terminal_sessions`), "Show in picker" toggle (`:249-285`, global or per-project `hidden`, locked when hidden globally);
   - description: "N models available." or `kind.unavailable_hint()` (`:287-300`);
   - first row carries search id `session-surface` (`:302-303`);
   - loading: `note("Checking installed CLIs…")` inserted at top while `discovery_loading` (`:305-307`).
4. **Background tasks** `background-model` (`:309-371`). Unchanged.
5. **Advanced**: `idle-agents`, `claude-hooks`, `claude-entrypoint` (`:372-375`). Unchanged.

Per PRD, from block 3 the following go away or change: the project scope select and all `project_providers` writes (PRD: per-project defaults no longer apply), the "Use by default" button (moves to row menu), the INSTALLED pill (becomes grouping), "Show in picker" (becomes the row enable switch, global only), per-row segmented (only under "Choose per agent"), `folder-open` → CLI details popover (folded into the expanded row's Command field).

Data used: `self.discovery.binaries: Vec<(Kind, Option<PathBuf>)>` (`data.rs`, filled by `SettingsView::refresh`), `self.ctx.catalogs`, prefs `binary_paths`, `provider_surfaces`, `default_provider`, `default_models`, `hidden_providers`, `project_providers` (`settings/preferences.rs:102-118`).

### 3.2 `settings/management.rs` — CLI details (`Target::Cli(Kind)`)

- `Target::Cli` `:8`; `open_management` `:34-130` creates `Input::field("CLI path", "Auto-detected path")` prefilled from `prefs.binary_paths`, then `inspect_cli(kind, false)`.
- `inspect_cli(kind, save)` `:142-185`: background `monocode_core::cli::inspect(kind, Some(&draft))` → `Inspection { path, version }`; on `save` writes/removes `prefs.binary_paths[kind]` and closes. Generation-guarded by `management_generation`.
- `submit_management` `:338-347` routes CLI to `inspect_cli(kind, true)`.
- Form `management_form` `:466-532`: header "X CLI" + status "Global path · Needs attention / Restart required / Configured / Auto-detected" (`pending` = `monocode_core::cli::configured(kind)` != saved path); read view shows mono path + version box; edit view `form_field("CLI path")` + note "Changes apply after restarting Monoptah."
- Actions `:938-995`: Retry (on error), Open location (`cx.reveal_path`), Edit path, Use auto-detected path, Save path.
- Overlay `management_overlay` `:1084-1105`: CLI details render as a `popover("cli-details", Anchor::Slot(binary_anchors[kind]))`, width 440, not a modal.

Reusable logic for the new Command field: `inspect_cli` (validation + version), the status vocabulary, `cli::configured` restart-pending check, `reveal_path`. The popover UI, `binary_anchors` and the `Target::Cli` arm of the shared `Editor` are what the inline field would replace; `Target::Mcp/Skill/Worktree/DeleteTree(s)/ResetMono` stay.

---

## 4. Settings search and layout tests

### 4.1 Search (`settings/model.rs`)

- `INDEX: &[(id, Section, label, keywords)]` `:184`; Providers entries `:474-528`: `agent-clis`, `session-surface`, `background-model`, `provider-accounts`, `show-remaining-usage`, `mask-emails`, `claude-hooks`, `claude-entrypoint`, `idle-agents`.
- `Section::keywords` Providers `:145-147`: "account sign in login model harness claude codex gemini cli default hooks".
- `search(query, compositor)` `:605-645`: label prefix 0, label contains 1, keywords contain 2; sections +0.5; top 8.
- Landing: `SettingsView::go` → `reveal_setting` (`settings/mod.rs:700-724`) sets `revealed`, scrolls to `targets[id]` (`scroll_to_revealed` `:737-751`) and flashes for 1.8s. A target exists only if some rendered element called `reveal_probe(id)` — via `row(Some(id),..)`, `group(Some(id),..)`, or directly (`pages/mobile.rs:778`). The `flashing(id)` check colours the row/group.
- Test: `model.rs:682-722` asserts "session surface", "terminal", "tui" find `Providers`/`session-surface`.
- New searchable rows need: an `INDEX` entry, a `'static` id passed to `row`/`group`/`reveal_probe`, and (if toggle-only) membership in `row`'s `toggle_only` list (`widgets.rs:290-322`) to avoid stacking under 560px.

### 4.2 Layout tests (`settings/layout_tests.rs`, declared from `settings/mod.rs`)

- Fixture `window(section, cx)` `:7-36`: 1200x800 window, `SettingsView::new(section, Preferences::default(), SettingsContext::default())`, discovery disabled (`discovery_task = None; discovery_loading = false`).
- Providers tests: `each_provider_chooses_the_surface_its_new_sessions_open_in` `:199-244` (asserts `settings-row-session-surface` exists and every `settings-session-surface-{kind}-{chat|tui}` selector, clicks write `prefs.provider_surfaces`); `the_surface_control_gives_way_to_a_hint_when_terminals_cannot_be_stored` `:247-275`; `the_default_badge_follows_the_account_chosen_with_the_pin_button` `:886+` (accounts). Both surface tests encode the current per-row segmented and will need rewriting for the new open-mode design.
- Pattern: find with `visual.debug_bounds("selector")`, `simulate_click(bounds.center())`, `run_until_parked`, read prefs with `window.update(visual, |view, _, _| view.prefs..)`. Click helper `click(visual, selector)` `:646`.

---

## 5. Light/dark theme handling for permission colours

- Light vs dark is decided in `configure_project` (`theme.rs:72-160`): `light = p.theme == "light" || (p.theme == "system" && system_light)`; ink and accent come from the selected workspace palette (`settings/palettes.rs`, 11 presets with separate dark/light `Palette { surface, panel, solid, content, accent }`) or the user's custom accent.
- The established light-aware pattern for semantic colours is a fn returning `rgb(if is_light() { darker } else { lighter })`: `warning_color`, `success_color`, `danger_color` (`theme.rs:325-337`). `text_selection` (`:303`) and `composer_shadow` (`:309`) also branch on `is_light()`. `modal.rs:340-342` `danger(alpha)` uses HSL with lightness by `is_light()`.
- Tinted fills elsewhere: `accent_color().opacity(a)` / `Hsla.a = alpha` (e.g. `widgets.rs::accent`, `conversation.rs:125 tint`).
- Not light-aware today: `settings/widgets.rs:13 RED 0xf87171` (`red()`, `secondary_button` danger), `management.rs` error text `rgb(0xf87171)`, `conversation.rs:120-124` Tailwind `YELLOW_300/SKY_300/FUCHSIA_300/AMBER_400`, `notifications/errors.rs` rose/amber.
- Design token values (`.ryugi/.../pages/tokens.css`): dark own `#a3a3a3`, ask `#4da3f5`, edits `#3fbfa8`, auto `#a78bfa`, full `#f2655b`; light own `#6b6b6b`, ask `#1f7fd8`, edits `#13917c`, auto `#7c5cd6`, full `#d6453b`. Badge/icon-tile bg = colour at .13, warning bg = full at .10. Design's dark "ask" equals `ACCENT 0x4da3f5`; design's light accent `#1f7fd8` differs from the app's light default palette accent (`palettes.rs` "default" light `accent: 0x4da3f5`). Design's dark `--red #f2655b` differs from `danger_color()` dark `0xf87171` and light `#d6453b` from `0xdc2626`.
- No permission-level colour functions exist in `theme.rs` today.

---

## Caveats / Not Found

- No shared tooltip, badge/tag, toast host for Settings, skeleton, inline banner, radio-card, two-column field grid, or settings text-field wrapper exists; each is built locally today in the places cited.
- `segmented` has no icon support (explicit comment `providers.rs:224-225`).
- `Input` emits no blur event; blur-saving needs `cx.on_blur` on the input's focus handle.
- `MenuAction` supports one description line; the design's extra mono flag line per permission option has no field.
- Design metrics (toggle 32x18, card radius 10, segmented filled style, row min-height 56) differ from the existing widgets (toggle 36x20, `rounded_xl` card, hairline segmented, row py 14); the existing widget metrics are what the rest of Settings uses.
- `.trellis/spec/native/` has no `examples/` directory (listed in the spec tree prompt, absent on disk).
