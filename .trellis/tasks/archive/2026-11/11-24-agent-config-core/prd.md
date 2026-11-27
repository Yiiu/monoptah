# Agent config core: data, migration, launch wiring

Child 1 of `11-24-agent-settings-redesign`. Read the parent's `prd.md`, `design.md` (§1–§6, §8,
§9) and `implement.md` ("Child 1"); they are the requirements. Research: parent `research/`.

Native only (`crates/monocode-core`, `crates/monocode-gpui`). No visible change to the settings
page in this child: the old controls keep working on top of the new storage until child 2.

## Acceptance criteria
- [ ] `AgentPreferences` stored under `preferences.agents`; old saved state migrates once per
      design §3 (effort, permission from `default_access`, open mode, disabled default cleared);
      re-loading is a no-op; a bad value in a new field does not reset the workspace.
- [ ] `Preferences::agent(kind)` is the only way launches read per-agent config.
- [ ] `AccessMode::AgentDefault` ("own") passes no permission flag/params on every path.
- [ ] Args (shell-split) and env reach chat and TUI launches of every agent; account env wins on
      collisions; warm-process key includes them.
- [ ] Claude/Codex permission values verified against the installed CLIs; mapping fixed if rejected.
- [ ] CLI path changes apply without restart.
- [ ] New sessions start from settings; composer choices are session-only; `project_providers`,
      `default_access`, `provider_efforts` no longer read or written after migration.
- [ ] New-session picker shows when ≥2 agents are available and any resolves to TUI.
- [ ] Core + targeted GPUI tests pass; fmt and clippy clean.
