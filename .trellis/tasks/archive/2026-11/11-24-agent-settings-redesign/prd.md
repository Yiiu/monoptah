# Redesign the Agent CLIs settings section

## Goal

Make the native app's agent settings work like Orca's Agents page: detecting, installing and
configuring every agent happens in one place, with more per-agent options and clearer
interactions. Scope is the **Agent CLIs** block of Settings → Providers
(`crates/monocode-gpui/src/settings/pages/providers.rs`). The React `ProvidersPage` is not changed.

Sources: Codex grill session 2026-11-24 (`~/.codex/sessions/2026/11/24/rollout-2026-11-24T11-42-56-*.jsonl`),
Claude Code grill session 2026-11-24, Orca source (`stablyai/orca`,
`src/renderer/src/components/settings/agents/`).

## Decided

### Scope and model
- Native only: `crates/monocode-gpui` and `crates/monocode-core` (the native workspace). No
  changes to the Tauri app (`src-tauri/`), the React app (`src/`), `web/` or `host/`.
- One long-lived configuration per agent. No named profiles, no per-project overrides.
- Existing per-project provider defaults (`project_providers`) stay in the data but no longer
  apply; new sessions use this page's global configuration.
- Chat and terminal (TUI) sessions read the same configuration; each applies the fields it supports.
- No custom (user-defined) agents; the agent list stays the fixed `Kind` enum.
- Install and update keep the current logic; only the page changes.
- Accounts, usage/privacy and the other Providers groups stay where they are.

### Per-agent fields (only these)
Command, model, reasoning effort, permission, launch arguments, environment variables.
The Codex history import path is dropped from this task: no import exists in the code today, so
it would be a new feature (separate task later).

### Permissions
- One global default permission, with optional per-agent overrides; an overridden row shows a badge.
- The permission icon and colour come from one shared function, used by both this page and the
  composer's access menu, so a level looks the same everywhere.
- Options follow Orca: "Use <agent>'s own settings" (passes no flag), Ask, Accept edits,
  Approve for me, Full access. An agent hides the modes it does not support.
- Migration seeds the global permission from the saved `default_access` when present, so
  existing users see no behaviour change; only users with no saved access start on
  "Use the agent's own settings".

### Launch arguments and conflicts
- One line, split with shell rules. An argument line that cannot be split (e.g. unclosed
  quote) is rejected. The split result is shown below the field.
- Typing a known permission / model / effort flag into the arguments moves it into the matching
  structured field and shows a notice (Orca's behaviour). Other arguments pass through unchanged.
- Launch arguments and environment variables apply to chat and terminal (TUI) sessions of every
  agent. Model, effort and permission apply to TUI sessions only where the flag is verified
  (Claude; Codex after checking the installed CLI); elsewhere the TUI does not receive them, and
  the launch preview (which follows the agent's open mode) shows exactly what is passed.
- When a user variable collides with one the selected account sets or removes (`CODEX_HOME`,
  `CLAUDE_CONFIG_DIR`, account tokens), the account wins; the editor marks that row
  "Set by the current account".

### Environment variables
- Read view: one row per variable. Values whose name contains KEY, TOKEN, SECRET, PASSWORD or
  AUTH are masked until revealed; other values are shown. "Edit as text" switches to a
  `KEY=VALUE` text editor. Values are stored in plain text in the existing settings store;
  moving secrets to an OS credential store is out of scope. This is a deliberate, documented
  exception to the core persistence rule against plain-text tokens (see `design.md`).

### Session overrides
- Changing model, permission or effort inside a session affects that session only. New
  sessions always start from this page. "Remember the last composer choice" ends;
  `provider_efforts` and `default_access` are read once at migration as initial values.

### Saving
- Toggles and selects save immediately.
- Text fields (command, arguments, environment, import path) validate and save on blur. On a
  failed validation the field is marked and the previous value stays in effect.
- A command path change takes effect immediately for new sessions (the CLI path map becomes
  reloadable); there is no "restart required" state any more.

### Layout and interaction
- Top of the section: default open mode and default permission.
- Two groups: "Installed (N) + Refresh" and "Available to install" (dimmed). An undetected
  agent's expanded row shows only the command path field.
- Each agent is one expandable row; only one row is expanded at a time.
- Collapsed row: icon, name, Default badge, permission badge when overridden, Chat|TUI switch,
  enable switch, and a monospace summary of the effective command, arguments and env.
- No "launch now" button on this page.

### New session flow
- If any available agent is set to TUI, the existing agent picker is shown on new session;
  otherwise the default agent opens directly. (Today the picker shows only when the default
  agent is TUI and there are at least two candidates, `shell/provider_prompt.rs:28`.)

### Open mode (Chat / TUI)
- Top selector: Chat / TUI / Choose per agent. Row Chat|TUI switches show only under
  "Choose per agent"; otherwise every agent uses the top value. Migration picks
  "Choose per agent" when existing per-agent surfaces differ, keeping each row's value.

### Enable and default
- "Show in picker" (`hidden_providers`) becomes the row's enable switch. A disabled agent is
  hidden from pickers and cannot be the default. Disabling the default agent clears the
  default (fallback: most recent, then Codex) and shows a notice.

### Model
- First option "Agent default" (passes no `--model`), the default for new agents; usable even
  when the model list has not loaded (fixes "Use by default" being disabled then).
- A custom model name may be typed; it is passed as `--model` verbatim and labelled "Custom".

### Unsupported fields
- Fields an agent does not support are hidden. `design.md` holds the agent × field support table.

### Delivery
- Parent task with two children: (1) data model + migration + launch wiring (no UI change),
  (2) settings page UI. A clickable HTML mockup and a Ryugi design draft are reviewed before
  `design.md` is written.

### Visual design (approved 2026-11-24)
- Ryugi artboard `artboard/agent-clis-settings` (v2) is the approved design: permission levels
  with their own colour and icon (own = grey, ask = blue, accept edits = teal, approve for me =
  violet, full access = red, with a warning under the picker), "Opens in" cards in the expanded
  row, effort ladder, launch preview with copy, per-field "Saved", open-mode cards with
  previews, detection skeleton and error banner. No "N agents set differently" summary.
- Implementation follows the project's design rules strictly: reuse existing shared components
  (settings widgets, `popover.rs`, `modal.rs`, theme tokens); anything new is built in the same
  style and placed in the shared component layer. Refined, elegant, simple.

## Open
None from the grill session.

## Acceptance criteria
To be written once the open questions are settled.
