# Component Guidelines

> How views are built in `crates/monocode-gpui`, and the shared pieces to reuse instead of
> re-implementing. Fix shared behavior in the shared component so every caller gets it.

## Views and entities

- A view is a struct with `impl Render`, created with `cx.new(|cx| ...)` and held as
  `Entity<T>` by its owner (`Shell` holds `settings: Option<Entity<SettingsView>>`,
  `sessions: HashMap<i64, Entity<Conversation>>`, `usage_footer`, ...).
- Children talk to owners through **events**: `impl EventEmitter<E> for View {}` +
  `cx.emit(..)`, and the owner keeps the `Subscription` from `cx.subscribe`/`subscribe_in`.
  Example: `SettingsView` emits `SettingsEvent` (`settings/mod.rs`), handled in
  `shell/settings.rs` (`SettingsEvent::Changed(prefs) => { ... this.persist(cx) }`).
  Store subscriptions in a field named `_events`/`_subscriptions` so they live with the view
  (`shell/actions.rs` `ActionDialog::_events`, `popover::Choices::_events`).
- Event handlers use `cx.listener(|this, event, window, cx| ...)`; call `cx.notify()` after
  changing state that affects rendering.
- Stateless building blocks are plain functions returning `Div`/`Stateful<Div>` or
  `impl IntoElement` (`modal::card()`, `theme::icon_button(..)`), or `RenderOnce` builders
  (`popover::Popover`). Prefer these over new entities for pure presentation.
- Expensive children are rendered with `.cached(StyleRefinement::..)` so typing does not
  re-render them; `main.rs` `native_tests::long_history_virtualizes_and_typing_does_not_rebuild_it`
  asserts that typing never re-renders the transcript.
- Give interactive elements an `.id(..)`, a `Role` and an `aria_label` (`modal::button`,
  `theme::chrome_button`). Labels and accessibility text are English.
- Elements a test must find get `.debug_selector(|| "name".into())`
  (`shell/rails.rs`: `format!("sidebar-session-{id}")`).

## Shared components to reuse

### Modal dialogs — `src/modal.rs`

| Need | Use |
| --- | --- |
| Card with frosted surface, border, shadow | `modal::card()`, `card_body()`, `title()`, `body()`, `actions()` |
| Buttons | `primary_button`, `ghost_button`, `danger_button`, `danger_ghost_button`, `disabled(button, bool)` |
| Option rows | `check_row`, `radio_row` |
| Scrim + centered card + dismiss on outside press | `modal::overlay(id, card, on_dismiss)` |
| Lift a pane-rendered overlay to cover the whole window | `modal::window_overlay(window, view)` (priority 30) |
| Block every hitbox behind a dialog | `modal::barrier(id, priority, window)`; blocking progress dialogs use `BLOCKING_PRIORITY` (105) |
| Confirmations | `window.prompt(..)`. `main.rs` installs `cx.set_prompt_builder(modal::prompt_builder)`, so prompts render as the same card; Escape picks Cancel, Enter the first non-cancel answer |

Light themes paint the card opaque; macOS and Windows get backdrop blur, Linux gets the opaque
fill (`modal::surface`). Do not hand-roll a card.

### Popovers, menus and selects — `src/popover.rs`

- `popover(id, Anchor)` builds an anchored card with viewport flipping (`place`), entrance
  animation, outside-press dismissal (`on_dismiss`) and layered priorities (default 40;
  a popover built inside another is raised above its host through `HostLayer`).
  `.modal_surface(classic)` gives it the modal card look (used by `shell/actions.rs`
  `ActionDialog`).
- Anchor to a trigger with `AnchorSlot`: render `slot.probe()` inside the trigger, then
  `Anchor` from the slot. Context menus anchor to a point.
- `Menu` (+ `menu_popover(&menu, anchor, cx)`) is the keyboard-driven menu: it takes focus on
  open, handles arrows/Enter/Escape/Left/Right through the `PopoverMenu` key context bound in
  `popover::bind`, and returns focus on pick or dismiss. It emits `MenuEvent::Picked`/`Dismissed`.
  Searchable menus use `MenuSearch`.
- `Choices<T>` is the single-choice dropdown: `Choices::open(label, choices, width, window, cx,
  on_event)`, hold it in an `Option<Choices<T>>` while open, and close it with
  `choices.close(window, cx)` (dropping alone leaves focus on the closed list). Used by
  `cloud_settings/mod.rs`, `emoji_library/view.rs`, `monos/emoji_ui.rs`.
- Settings pages use their own `select`/`segmented`/`toggle`/`slider` helpers in
  `settings/widgets.rs`; use those inside Settings.

### Theme and colors — `src/theme.rs`

- Colors come from thread-local tokens configured from `Preferences` and the project theme
  (`configure_project`, with a 360 ms tween between projects). Read them through functions,
  never hardcode hex in views: `content(alpha)` for ink at an opacity, `accent_color()`,
  `pane_fill()`, `shell_fill()`, `floating_fill()`, `danger_color()`, `warning_color()`,
  `success_color()`, `is_light()`.
- Interaction fills are translucent ink (`content(0.07)` on hover), not opaque greys (comment
  at the top of `theme.rs`).
- Shared metrics are constants: `TITLE_BAR_HEIGHT`, `PANE_HEADER_HEIGHT`, `TAB_PLATE_HEIGHT`,
  `COLUMN`, `FOOTER_TEXT_SIZE`.

### Icons — `src/theme.rs`, `src/assets.rs`, `src/file_icons.rs`

- `icon(name, side)` draws an SVG by asset name tinted with `content(0.5)`;
  `icon_button(id, label, path)` / `icon_button_enabled(..)` is the 24 px chrome button.
- A name resolves in `Assets::load` (`assets.rs`): an explicit arm with `include_bytes!`, the
  `mi/` prefix for file-type icons (`file_icons::svg`), or a stroke body in `icon_body`
  wrapped in a 24x24 `stroke='white'` SVG. Add new icons there; unknown names render nothing.
- File and folder icons: `file_icons::file_type_icon(name, is_dir, open, side)`.
  Provider logos: `theme::provider_icon(provider, side)`.

## Actions and keybindings

- Declare actions with `actions!(namespace, [..])` (`shell.rs` `workspace`, `popover.rs`
  `popover_menu`); actions with data derive `gpui::Action` (`shell.rs` `ActivateTab(pub isize)`).
- Bind keys in a module `bind`/`bind_keys(cx: &mut App)` called from `main.rs`. Always scope
  bindings to a key context: `shell::bind_keys` uses `Some("Workspace")` because "GPUI ranks an
  unscoped binding above every context", so the focused composer, menus and pickers outrank
  workspace shortcuts. Complex predicates are allowed
  (`"Session > (Composer || TranscriptSelection) && !CodeEditor && !PopoverMenu && !Prompt"`).
- Production bindings pick the platform modifier explicitly
  (`let modifier = if cfg!(target_os = "macos") { "cmd" } else { "ctrl" }`); macOS-only extras
  are `#[cfg(target_os = "macos")]` (`cmd-l`, `cmd-q` in `main.rs`). Tests type
  `secondary-...` keystrokes, never `cmd-...` (see [testing.md](./testing.md)).
- User overrides are applied by `settings::apply_keybindings` (`settings/keymap.rs`), which
  snapshots the default bindings once and rebuilds from them keeping each binding's predicate.
  A new bindable command must be reachable through its `command()` mapping to appear there.
- Handlers sit on the element that owns the key context: the `Shell` root
  (`.key_context("Workspace").track_focus(&self.focus).on_action(cx.listener(..))`).
  While a blocking modal is open, `shortcuts::block_menu_actions` disables menu actions.

## Focus

- Focusable views hold a `FocusHandle` from `cx.focus_handle()`, implement `Focusable`, and put
  `.track_focus(&self.focus)` plus `.key_context("Name")` on their root (`modal::Prompt`).
- Move focus with `window.focus(&handle, cx)` (this GPUI version takes `cx`).
- A transient surface returns focus where it came from: `Menu` restores it on pick/dismiss,
  `Choices::close` refocuses, the shell refocuses the active composer
  (`shell.update(cx, |shell, cx| shell.focus_active(window, cx))` in `main.rs`).
- Native web views paint above GPUI, so the shell hides them while a blocking dialog or
  lightbox is open (`m.set_suppress_html(covered)` in `impl Render for Shell`).
