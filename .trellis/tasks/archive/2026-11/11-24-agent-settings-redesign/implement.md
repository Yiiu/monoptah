# Implementation plan

Two child tasks, run in order. Each is planned, built, checked and merged on its own (one PR each,
squash). Child 2 depends on child 1 being merged.

Build rules (CLAUDE.md "Developing the native app"): seed the build cache in a new worktree;
native workspace commands with the `native-dev` profile (`cargo ntest`, `cargo nclippy` from
`crates/monocode-gpui`); make all edits first, then one test pass filtered to touched modules;
GPUI tests Linux-portable.

## Child 1 — `agent-config-core`: data, migration, launch wiring (no visible page change)

1. Core `agent.rs`: `AccessMode::AgentDefault` ("own"); handle it in `codex()`, `claude()`,
   `lifecycle::codex_access`, ACP mapping, OpenCode rules, Devin config, `tui_launch::build`.
2. Core: `launch_args` module — `split(line)`, `extract_structured(kind, args)`, `parse_env(text)`,
   `is_secret(key)`; unit tests.
3. Core: inject args/env in `run_operation` and `opencode::command()` (order per design §5);
   include them in `parked::key`; TUI `build`/`prepare` take args/env; `Profile::owned_keys`.
4. Core: verify Claude `--permission-mode` and Codex `-a` values against the installed CLIs; fix
   mapping if rejected; Codex TUI flags only if verified (`tui_caps`).
5. Core: `cli::initialize` reloadable (drop `INIT`).
6. GPUI `preferences.rs`: `AgentPreferences`, `Preferences::agent(kind)`, surface resolver;
   migration in `Workspace::restore`; tests.
7. GPUI: seed new sessions from the accessor (`session_loading.rs`, `conversation.rs:810`,
   `quick_composer`, `app_views`, `mobile_host`, `cloud_start`, `automations`, `monos`); remove
   remember-last writers and `project_providers` reads; new picker rule; rewrite
   `conversation.rs:4987/5018`, surface tests.
8. Check: `cargo fmt`, core clippy + tests, `cargo nclippy`, `cargo ntest` filtered to
   preferences/workspace/new_chat_choice/surface/provider_prompt/conversation tests.

Rollback point: revert child 1's PR; page still shows the old controls.

## Child 2 — `agent-settings-page`: the page

1. Icons in `assets.rs` (design §7 list).
2. `theme.rs`: `permission_color`, `permission_icon`.
3. `popover.rs`: `MenuAction::detail` (mono second line) for `MenuStyle::Access`.
4. `settings/widgets.rs`: segmented icons, `radio_cards`, `badge`, `level_picker`, `field_grid`,
   `status_line`, `saved_flash`, `code_block`, `inline_banner`, `skeleton_row`, `env_table`,
   settings toast host.
5. `settings/pages/providers.rs`: rebuild the Agent CLIs group per artboard v2 (defaults card,
   Installed / Available groups, expandable rows one at a time, ⋯ menu, Command field with
   inspection, model, effort, permission, args, env, launch preview, toasts); remove scope select,
   CLI details popover (`management.rs` `Target::Cli`), `project_providers` writes.
6. `settings/model.rs`: search entries and keywords; `row` `toggle_only` list if needed.
7. Layout tests per design §9; rewrite the two surface tests.
8. Check: fmt, `cargo nclippy`, `cargo ntest settings::`; `npm run native:check:mac` is not
   required on Windows. Run Native Dev and compare the page against the artboard in light and
   dark themes; the user does the final visual review.

Review gate per child: implementer hands over a checklist mapping each PRD point to `file:line`
and its test; one review per request (CLAUDE.md).

## After both
- Spec update: record the plain-text env exception in `.trellis/spec/core/persistence-and-compat.md`;
  add the new shared widgets to `.trellis/spec/native/component-guidelines.md`.
