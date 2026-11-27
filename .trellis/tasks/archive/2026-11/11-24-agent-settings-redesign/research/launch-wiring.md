# Research: launch wiring for per-agent configuration

- **Query**: For each of the 11 `Kind` agents, how chat and TUI sessions launch today, which of
  model / effort / permission / args / env each path can apply, where args and env would be
  injected, how `AccessMode` / `provider_efforts` / `default_access` feed launches, the CLI path
  `OnceLock`, the new-session picker rule, and any existing Codex history import.
- **Scope**: internal (plus local `--help` of the installed `codex` 0.172.0, `claude` 2.1.331, `omp`)
- **Date**: 2026-11-24

All paths are relative to `crates/`. Line numbers are from the `chore/adopt-trellis` checkout.

---

## 0. Files found

| File | Role |
|---|---|
| `monocode-core/src/provider/kind.rs:3-15, 17-29, 86-91` | `Kind` enum, `ALL` order, `is_acp()` (everything except Codex/Claude/Pi/Omp), `is_pi()` |
| `monocode-core/src/provider.rs:84-92` | `Request { kind, binary, cwd, prompt, provider_id, model, settings }` |
| `monocode-core/src/provider.rs:160-165` | `resolve(kind)`: `cli::configured` override first, else `resolve_auto` |
| `monocode-core/src/provider.rs:166-...` | `resolve_auto`: `MONOCODE_<ID>_BINARY` env, GUI PATH, install dirs |
| `monocode-core/src/provider.rs:1527-1760` | `run_process` / `run_operation`: the one chat spawn for Codex, Claude, Pi/Omp, ACP agents |
| `monocode-core/src/provider.rs:583-640, 646-696` | Codex `thread/start|resume` and `turn/start` params (model, effort, approval, sandbox) |
| `monocode-core/src/provider/lifecycle.rs:163-174` | `codex_access()` (AccessMode -> Codex tuple + network for operator) |
| `monocode-core/src/provider/acp.rs:51-97` | `acp::configure`: per-agent CLI args for ACP agents |
| `monocode-core/src/provider/acp.rs:265-276` | `acp_new`: Grok `_meta.yoloMode` / `autoMode` |
| `monocode-core/src/provider/acp.rs:277-328` | `acp_next`: queued config options, then `session/set_mode` |
| `monocode-core/src/provider/acp.rs:329-456` | `acp_options`: permission -> ACP mode id; model -> `set_config_option`/`set_model` |
| `monocode-core/src/provider/acp.rs:457-540` | `acp_settings`: effort/fast/thinking -> `set_config_option` (Grok: `set_mode`) |
| `monocode-core/src/provider/acp.rs:650-735` | `session/request_permission`: client-side auto-approve by AccessMode |
| `monocode-core/src/provider/pi.rs:25-64` | `pi::configure`: Pi/Omp CLI args |
| `monocode-core/src/provider/pi.rs:242-262, 390-420` | Pi `set_thinking_level`, `set_model` over RPC |
| `monocode-core/src/provider/opencode.rs:195-207, 306-327` | OpenCode `command()` and `serve_command()` (separate from `run_operation`) |
| `monocode-core/src/provider/opencode/client.rs:427-457, 469-514` | OpenCode permission rules, model/variant, agent |
| `monocode-core/src/provider/devin.rs:25-34, 300-...` | Devin supervised `--config`, model/effort folding |
| `monocode-core/src/provider/parked.rs:296-321` | warm-process reuse key (includes binary, model, settings, environment) |
| `monocode-core/src/agent.rs:7-83` | `AccessMode` + `codex()` and `claude()` mappings |
| `monocode-core/src/agent.rs:87-155` | `agent::Settings` (`model`, `effort`, `access`, `environment` `#[serde(skip)]`) |
| `monocode-core/src/accounts.rs:35-45, 90-182` | `CLAUDE_CODE_ENTRYPOINT` flag, `Profile::directory/home/env/apply` |
| `monocode-core/src/tui_launch.rs:25-122, 153-163` | TUI `LaunchInput`, `build()`, `tui_binary()` |
| `monocode-core/src/surface.rs:36-54` | `tui_caps()`: only Claude has resume / initial prompt / launch flags |
| `monocode-core/src/cli.rs:11-28` | CLI path override map behind `OnceLock` |
| `monocode-gpui/src/tui_session.rs:128-228, 490-522` | TUI `prepare()` and `Terminal::launch` call |
| `monocode-gpui/src/terminal.rs:293-314, 584-622` | PTY env: set list, removals via `/usr/bin/env -u` (unix only) |
| `monocode-gpui/src/runtime.rs:96-..., 197-...` | chat `run_turn` / `run_turn_using(resolve_binary=provider::resolve)` |
| `monocode-gpui/src/conversation.rs:804-814, 1651, 2533-2536, 2601-2616, 2845, 4093-4116` | composer model/access/effort seeding and "remember last" |
| `monocode-gpui/src/shell/session_loading.rs:104-200` | `NewSessionDefaults::read/create`: seeds a new session's stored `Settings` |
| `monocode-gpui/src/shell/new_chat_choice.rs:1-90` | provider + model resolver for a new session |
| `monocode-gpui/src/shell/provider_prompt.rs:1-117` | new-session agent picker rule |
| `monocode-gpui/src/settings/preferences.rs:95-120, 315-325` | `binary_paths`, `provider_surfaces`, `default_models`, `hidden_providers`, `project_providers`, `default_access`, `provider_efforts`, `hidden_for()` |
| `monocode-gpui/src/shell.rs:1144`, `shell/settings.rs:552-558` | `cli::initialize(binary_paths)`, `set_claude_entrypoint` |

---

## 1. Per-agent launch paths

### 1a. Chat sessions: common spine

Every local chat turn goes `runtime::run_turn` (`monocode-gpui/src/runtime.rs:96`) ->
`run_turn_using(..., provider::resolve)` (`runtime.rs:197`) -> `provider::run` (`provider.rs:1431`)
-> `run_operation` (`provider.rs:1537`). Other callers build a `Request` themselves:
`conversation/btw.rs:714`, `conversation/commands.rs:180`, `conversation/edit_last_turn.rs:243`,
`session_title.rs:181` (text-only background jobs). Remote (`remote://`) sessions go to
`runtime::remote::run` (`runtime.rs:105`) and never reach `run_operation`.

`run_operation` (`provider.rs:1569-1572`):

```rust
let mut command = Command::new(&request.binary);
crate::scm::shell::apply_gui_env(&mut command);
command.envs(&request.settings.environment);
request.settings.account.apply(request.kind, &mut command)?;
match request.kind { Kind::Codex => {...} Kind::Claude => {...}
    kind if kind.is_pi() => pi::configure(...), _ => acp::configure(...)? }
```

OpenCode branches off before this (`provider.rs:1551-1553` -> `opencode::run`), and builds its own
`Command` in `opencode.rs:195-207` with the same three env steps.

### 1b. TUI sessions: common spine

`TuiSession` -> `tui_session::prepare` (`monocode-gpui/src/tui_session.rs:128`):
reads the session's stored `agent::Settings` (`store.agent_settings(id)`, line 148), resolves the
binary via `tui_launch::tui_binary` (`tui_launch.rs:153` -> `provider::resolve`; Antigravity is the
literal `agy` on PATH, `tui_launch.rs:161-163`), computes account env with `profile.env(kind)`
(line 189), and calls `tui_launch::build(LaunchInput { kind, binary, model, access, ... })`
(lines 190-200). The PTY is started by `Terminal::launch(cwd, program, args, env_set, env_remove)`
(`tui_session.rs:514-521`).

`tui_launch::build` (`tui_launch.rs:66-122`) only adds flags when `tui_caps(kind).launch_flags`,
which is true **only for Claude** (`surface.rs:45-53`). So for every other agent the TUI is the bare
CLI with no arguments at all (no model, no permission, no resume, no initial prompt).

### 1c. Per-agent detail

| Agent | Chat transport | Chat: model | Chat: effort | Chat: permission | TUI today |
|---|---|---|---|---|---|
| **Claude** | stream-json CLI, `provider.rs:1594-1727` | `--model <m>` (+`[1m]` suffix for 1M context) `provider.rs:1714-1726` | `--effort <lvl>` (ultracode->`xhigh`+settings, model-specific remaps; `ultrathink` dropped) `provider.rs:1680-1704` | `--permission-mode <AccessMode::claude()>`, `plan` when plan turn; `--allow-dangerously-skip-permissions` when bypass; ReadOnly also `--tools Read,Glob,Grep,Bash` `provider.rs:1606-1667` | `--model`, `--permission-mode`, `--settings`(hooks)+`--setting-sources`, `--resume`/`--session-id`, prompt `tui_launch.rs:69-117` |
| **Codex** | `codex app-server` JSON-RPC, `provider.rs:1574-1592`, params `583-696` | `thread/start.model`, `turn/start.model` + `collaborationMode.settings.model` `provider.rs:613-615, 686-695` | `turn/start.effort` + `collaborationMode.settings.reasoning_effort` `provider.rs:661-663, 692` | `approvalPolicy`, `sandbox`, `sandboxPolicy`, `approvalsReviewer` from `AccessMode::codex()` via `codex_access()` (`lifecycle.rs:163-174`), on both thread and turn; plan turn forces readOnly/never `provider.rs:676-685` | bare `codex` |
| **Pi** | `pi --mode rpc`, `pi.rs:25-64` + RPC | `--model provider/id` only if it has a `/` (`pi.rs:57-63`); RPC `set_model` if different (`pi.rs:390-410`) | RPC `set_thinking_level` from `model_settings.thinking` or `effort` (`pi.rs:242-262`) | No permission flag. ReadOnly/plan -> `--tools read,grep,find,ls` (`pi.rs:37-46`). Confirmations come as RPC `confirm` and are always shown (`pi.rs:610-630`) | bare `pi` |
| **Omp** | same as Pi (`is_pi`) | same as Pi (but `set_model` only for `Kind::Pi`, `pi.rs:394`) | same | ReadOnly -> `--tools read,grep,glob,lsp` (`pi.rs:40-42`); resume `--resume` (`pi.rs:49-55`) | bare `omp` |
| **Grok** | ACP, `acp.rs:59-76` | `--model <m>` CLI flag (`acp.rs:65-67`); also `session/set_config_option`/`set_model` (`acp.rs:401-450`) | `--reasoning-effort <e>` (`acp.rs:68-70`) and `session/set_mode {modeId: effort}` (`acp.rs:483-494`) | ReadOnly/plan -> `--permission-mode plan` (`acp.rs:61-63`); FullAccess -> `--always-approve` (`acp.rs:71-73`) and `session/new _meta.yoloMode` (`acp.rs:267-270`); Auto -> `_meta.autoMode` (`acp.rs:271-273`); plus client auto-approve (`acp.rs:693-700`) | bare `grok` |
| **Cursor** | ACP `cursor-agent acp` (`acp.rs:83-85`) | `session/set_config_option configId=model` (`acp.rs:435-439`) | via advertised config options (`acp.rs:457-540`) | No mode (`acp.rs:381-383`); only client-side auto-approve in `request_permission` (`acp.rs:693-735`) | bare `cursor-agent` |
| **OpenCode** | own HTTP `serve` path, `opencode.rs` + `opencode/client.rs` | required `provider/model` (errors otherwise), `POST /session/{id}/model` (v2) or prompt body (`client.rs:465-480, 534`) | `model_settings.variant` -> `model.variant` (`client.rs:469-474, 535`) | session permission rules: FullAccess `*:allow`; else `*:ask` + `question:allow`; AutoAcceptEdits/Auto add `edit:allow`; Auto adds `read:allow` (`client.rs:427-457`); ReadOnly/plan -> agent `plan` (`client.rs:498-508`); `permission.asked` auto-answers (`opencode/events.rs:430-462`) | bare `opencode` |
| **Antigravity** | ACP `agy_acp_server.par acp`, cwd = binary dir (`provider.rs:1730-1734`, `acp.rs:54-58`) | only if advertised as config option (`acp.rs:446-448`) | advertised config options; booleans typed (`acp.rs:528-535`) | `session/set_mode`: FullAccess `yolo`, AutoAcceptEdits `auto_edit`, else `default` (`acp.rs:334-344`); client auto-approve edits (`acp.rs:697-705`) | `agy` on PATH (`tui_launch.rs:161-163`) |
| **Hermes** | ACP `hermes acp` (`acp.rs:92-94`) | `set_model` unless `default` or already current (`acp.rs:408-418, 449-454`) | only `effort` kept (`acp.rs:474-476`) | `session/set_mode`: Supervised `default`, AutoAcceptEdits `accept_edits`, else `dont_ask`; plan -> `default` (`acp.rs:345-355`) | bare `hermes` |
| **Fx** | ACP `fx acp --model <m>` (`acp.rs:86-91`) | `--model` flag + `set_config_option model` (`acp.rs:425-440`) | config options | mode `ask` (plan/ReadOnly) or `code` (`acp.rs:367-373`); **always** auto-approves (`acp.rs:693`, Fx -> `Decision::Session`) | bare `fx` |
| **Devin** | ACP `devin [--config <file>] acp` (`acp.rs:77-82`) | model folded with effort (`devin.rs:300-...`), only advertised choices (`acp.rs:419-432`) | folded into model or nearest advertised effort (`acp.rs:497-526`) | Supervised -> `--config <ask-before-edits file>` (`devin.rs:25-34`); mode: plan `plan`, FullAccess `bypass`, Auto `smart`, else `accept-edits` (`acp.rs:356-366`); approvals marked one-time unless FullAccess (`acp.rs:683-687`) | bare `devin` |

Note: `acp.rs:374-380` also maps OpenCode to `plan`/`build`, but OpenCode never reaches
`acp_options` because `run_operation` returns early into `opencode::run` (`provider.rs:1551`).

### 1d. Mapping the five planned permission levels onto existing mechanisms

The PRD levels are own / ask / accept-edits / approve-for-me / full-access. `AccessMode` today has
ReadOnly, Supervised, AutoAcceptEdits, Auto, FullAccess (`agent.rs:7-14`) and no "own settings"
value: every chat path above always sends some permission value (Claude always passes
`--permission-mode`, Codex always passes `approvalPolicy`/`sandboxPolicy`, Claude TUI always passes
`--permission-mode`).

| Level | Claude (chat & TUI) | Codex chat (app-server) | Codex TUI flag (0.172.0 `--help`) | ACP / others (existing mapping) |
|---|---|---|---|---|
| own | omit `--permission-mode` (not possible today) | omit approval/sandbox params (not possible today) | no flag | no mode / no auto-approve |
| ask | `default` (`agent.rs:79`) | `untrusted`, `read-only` (`agent.rs:51-56`) | `-a on-request -s read-only` (note: `untrusted` is no longer a listed `-a` value in 0.172.0) | Hermes `default`; Devin `--config` ask-before-edits; OpenCode `*:ask`; Antigravity `default`; Grok none |
| accept edits | `acceptEdits` | `on-request`, `workspace-write`, reviewer `user` (`agent.rs:57-62`) | `-a on-request -s workspace-write` | Antigravity `auto_edit`; Hermes `accept_edits`; OpenCode `edit:allow`; Devin `accept-edits`; Cursor/Grok client auto-approve non-execute |
| approve for me | `auto` | `on-request`, `workspace-write`, reviewer `auto_review` (`agent.rs:63-68`) | `--approve-for-me` | Grok `_meta.autoMode`; Devin `smart`; OpenCode `edit`+`read` allow; Hermes `dont_ask`; Cursor client auto-approve all |
| full access | `bypassPermissions` + `--allow-dangerously-skip-permissions` | `on-request`, `danger-full-access` (`agent.rs:69-74`) | `-s danger-full-access` or `--dangerously-bypass-approvals-and-sandbox` | Grok `--always-approve` + `yoloMode`; Antigravity `yolo`; Devin `bypass`; OpenCode `*:allow`; Hermes `dont_ask` |
| (read-only, current only) | `plan` | `never`, `read-only` | `-s read-only -a never` | Pi/Omp read tools; Fx `ask`; OpenCode agent `plan` |

Local `claude --help` (2.1.331) lists `--permission-mode` choices `acceptEdits, auto,
bypassPermissions, manual, dontAsk, plan`; `default` (what `AccessMode::Supervised.claude()` sends)
is not in the printed list. `omp --help` shows `--approval-mode=always-ask|write|yolo`,
`--auto-approve`, `--thinking=<level>`, `--model=`; Pi's chat path uses none of them today.
Flags of Grok, Cursor, OpenCode, Hermes, Fx, Devin and `agy` TUIs were not verifiable locally
(CLIs not installed).

### 1e. Agent x field support table (what the code can apply **today**)

Legend: Y = applied today; P = partially / indirectly; N = no mechanism today; "new" = would need
new code at the injection point named in section 2.

| Agent | Chat model | Chat effort | Chat permission | Chat args | Chat env | TUI model | TUI effort | TUI permission | TUI args | TUI env |
|---|---|---|---|---|---|---|---|---|---|---|
| Claude | Y flag | Y flag | Y flag (5 modes) | new (Command) | P (settings.environment + account) | Y flag | N | Y flag | new (`build`) | P (account only) |
| Codex | Y protocol | Y protocol | Y protocol (4 + RO) | new (Command; app-server subcommand first) | P | N | N | N | new | P (account) |
| Pi | P (`provider/id` only) | Y RPC | N (RO tools only) | new | P | N | N | N | new | N |
| Omp | P | Y RPC | N (RO tools only; CLI has `--approval-mode`) | new | P | N | N | N | new | N |
| Grok | Y flag+ACP | Y flag+ACP | Y flag+meta+client | new (order-sensitive: global flags before `agent ... stdio`) | P (+`GROK_NO_AUTO_UPDATE`) | N | N | N | new | N |
| Cursor | Y ACP | P (if advertised) | P client-side only | new | P | N | N | N | new | N |
| OpenCode | Y HTTP (required) | P (`variant`) | Y session rules | new (shared v2 `serve`, `opencode.rs:306-327`) | P (shared service started once) | N | N | N | new | N |
| Antigravity | P (if advertised) | P | Y ACP mode | new | P | N | N | N | new (`agy`) | N |
| Hermes | Y ACP | Y (effort only) | Y ACP mode | new | P | N | N | N | new | N |
| Fx | Y flag+ACP | P | P (always auto) | new | P | N | N | N | new | N |
| Devin | P (advertised choices) | P (folded) | Y `--config` + ACP mode | new | P (Devin is in `accounts::PROVIDERS`, but `Profile::directory` returns an error for kinds without profiles, `accounts.rs:94-96`, so only the default account adds env) | N | N | N | new | N |

---

## 2. Where extra args and env would go

### Chat

| Path | Single injection point | Notes |
|---|---|---|
| Codex, Claude, Pi, Omp, ACP agents | `run_operation`, `provider.rs:1569-1729`, after the per-kind `match` and before `.current_dir` (`provider.rs:1729`) | Order constraints: Codex args after `app-server` (`provider.rs:1575`) or before it as global `-c`; Grok's `agent --no-leader ... stdio` sequence (`acp.rs:64-74`); Claude sends `--settings` JSON itself (`provider.rs:1705`), a user `--settings` would conflict. |
| OpenCode | `opencode::command()` `opencode.rs:195-207` (used by `exec` and `serve_command`) | v2 `serve` is a shared service started once outside any thread (`opencode.rs:303-311`), so per-session env/args reach only threads with their own `serve` child. |
| Warm reuse | `parked::key` `provider/parked.rs:296-321` | Key already hashes `binary`, `model`, `settings` (serde fields), `settings.environment` and the entrypoint flag. New config not inside `Settings` (or skipped by serde) would not invalidate a parked process. |

Env today, chat (order = precedence, later wins):

1. `apply_gui_env` (login-shell PATH etc.), `provider.rs:1570`.
2. `command.envs(&request.settings.environment)` (`provider.rs:1571`). `Settings.environment` is
   `#[serde(skip)]` (`agent.rs:113-114`); it is filled with `app_environment` (visual bridge / app
   endpoint) at `conversation.rs:1651`, `commands.rs:127`, `edit_last_turn.rs:253`, `btw.rs:1832`,
   Mono context at `conversation/mono.rs:210`, `monos/mod.rs:958`, goal at `provider/goal.rs:279`.
3. `account.apply` (`accounts.rs:170-182`): `CLAUDE_CODE_ENTRYPOINT=monocode` when the
   experimental preference is on (`accounts.rs:171-173`, set by `shell/settings.rs:557`), then
   per-account set (`CLAUDE_CONFIG_DIR`, `CLAUDE_SECURESTORAGE_CONFIG_DIR`, account token var; or
   `CODEX_HOME`) and removals (`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`,
   `CLAUDE_CODE_OAUTH_TOKEN`; `OPENAI_API_KEY`, `CODEX_API_KEY`, `CODEX_ACCESS_TOKEN`)
   (`accounts.rs:128-168`). Default account: nothing (`accounts.rs:91-93`).
4. Per-kind overrides after that: Codex Mono `CODEX_HOME`/`CODEX_SQLITE_HOME` + `-c sqlite_home`
   (`provider.rs:1582-1591`); Grok `GROK_NO_AUTO_UPDATE=1` (`acp.rs:75`); OpenCode
   `OPENCODE_PASSWORD` (`opencode.rs:322-324`); visual/mono bridge config for Codex
   (`provider.rs:1576-1581`).

Conflict points: a user-supplied `CODEX_HOME`, `CLAUDE_CONFIG_DIR` or API key placed at step 2
is overwritten or removed by step 3 when a non-default account is selected; with the default
account it would pass through. A user env applied after step 3 would instead override the account.

### TUI

| Path | Single injection point |
|---|---|
| args | `tui_launch::build` `tui_launch.rs:66-122` (returns `TuiLaunch.args`); or `prepare` before `Terminal::launch` (`tui_session.rs:190-200`). Claude prompt is last, after `--` when it starts with `-` (`tui_launch.rs:106-114`), so extra args must come before it. |
| env | `prepare`: `let (env_set, env_remove) = profile.env(kind)?;` `tui_session.rs:189` -> `Prepared.env_set/env_remove` -> `Terminal::launch` (`tui_session.rs:514-521`). |

TUI env today is **only** the account's `profile.env(kind)`: no `Settings.environment`, no
`app_environment`, and no `CLAUDE_CODE_ENTRYPOINT` (that is set only inside `Profile::apply`,
which the TUI does not call). Removals are done by wrapping the program in
`/usr/bin/env -u KEY -- program` on unix; on Windows removals are ignored (`terminal.rs:293-314`).
`accounts.rs:161-165` already special-cases this: the account token var is dropped from the
removal list because the `env -u` wrapper would also strip the value being set.

Shell-splitting: `shlex = "2"` is already a dependency of `monocode-core` (`monocode-core/Cargo.toml:14`)
and is used in `contributions.rs`, `git/report.rs`, `mono.rs`, `tool/presentation.rs`.

---

## 3. AccessMode, `default_access`, `provider_efforts` today

Preferences (`monocode-gpui/src/settings/preferences.rs:107-118`):
`default_models`, `hidden_providers`, `project_providers`, `default_access: Option<String>`
("The last access mode picked in a composer"), `provider_efforts: HashMap<String,String>`
("Last explicit effort choice for each provider").

Writers ("remember last composer choice"):

- `conversation.rs:2533-2536`: composer `access:` action sets `settings.access` and calls
  `remember_access` (`conversation.rs:4100-4106`) which writes `prefs.default_access`.
  Also `conversation/mono.rs:550`.
- `conversation.rs:2601-2616` `set_effort_choice`: writes `prefs.provider_efforts[provider]`.

Readers (seed a new session):

- `default_access(cx)` `conversation.rs:4093-4099`; `preferred_effort(provider, cx)`
  `conversation.rs:4113-4116`.
- New conversation view: `conversation.rs:810-814` (`access: default_access`, `effort:
  preferred_effort`); model from env `MONOCODE_<PROVIDER>_MODEL` (`conversation.rs:804-808`).
- Model change resets effort to `preferred_effort` (`conversation.rs:2845`).
- `NewSessionDefaults::read` `shell/session_loading.rs:140-157` (kind, model from
  `NewChatChoice`, account, `default_access`, `preferred_effort`, surface) and `create`
  `session_loading.rs:160-200`, which also merges `desktop_preferences.session_defaults`
  (React-side `monocode.projectProviderSettings.v1`, `monocode.defaultModels`,
  `monocode.lastModel`, `monocode.lastModelSettings`; `monocode-core/src/desktop_preferences/session_defaults.rs:10-40`)
  and saves the session's `agent::Settings` via `store.save_agent_settings` (line ~200).
  Callers: `shell.rs:4242`, `shell.rs:5358`.
- Other seeders: `shell/app_views.rs:585-592, 656-660` (default_access / preferred_effort),
  `shell/quick_composer.rs:39-47, 327, 374, 1775`, `shell/mobile_host.rs:709, 2533-2569, 2991`
  (remote devices; `remote_default_access` caps FullAccess, `mobile_host.rs:479`),
  `shell/mobile_host/cloud_start.rs:157-158`, `automations.rs:355-363` (project_providers +
  default_models), `monos/mod.rs:899` (default_models).
- Model resolver: `shell/new_chat_choice.rs:28-35` (`project_providers[cwd].models[kind]`, then
  `default_models[kind]`); quick composer duplicates it at `quick_composer.rs:356-366`.

Where the session-only override sits: the per-session `agent::Settings` (`model`, `effort`,
`access`, `model_settings`) persisted with `store.save_agent_settings` / `configure_agent`
(`runtime.rs` ~line 297, `locked(&store)?.configure_agent(summary.id, &summary.provider, &persisted)`).
Chat reads it per turn; TUI reads it at `tui_session.rs:148` (`store.agent_settings(id)`), so
TUI uses the same stored `model`/`access` the seeders wrote.

---

## 4. CLI path `OnceLock` (`monocode-core/src/cli.rs:11-28`)

```rust
fn paths() -> &'static RwLock<HashMap<String, String>> { static PATHS: OnceLock<RwLock<..>> ... }
pub fn initialize(overrides: &HashMap<String, String>) {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| { *paths().write().unwrap() = overrides.clone(); });
}
```

Confirmed: the map itself is an `RwLock`, but `initialize` writes it only on the first call
because of the `INIT` `OnceLock`. It is called at startup (`monocode-gpui/src/shell.rs:1144`) and
again on every preferences apply (`shell/settings.rs:552`), where the second call is a no-op. The
Settings row detects "pending restart" by comparing `cli::configured(kind)` with
`prefs.binary_paths` (`monocode-gpui/src/settings/management.rs:467-471`).

Reloadability: the storage is already an `RwLock`, so dropping the `INIT` guard would make
`initialize` overwrite on each call. Other state that would then see a changed path:
`provider::resolve` is uncached (`provider.rs:160-165`); `discovery.rs:25` caches identity checks
by (path, mtime); warm processes are keyed by `request.binary` (`parked.rs:306`), so a new path
does not reuse an old process; the "pending restart" UI (`management.rs:467-471`) and the shell's
`installed_providers` probe would need refreshing. Not changed, report only.

---

## 5. New-session agent picker rule

`monocode-gpui/src/shell/provider_prompt.rs`:

- `new_session_providers(hidden, installed)` (lines 15-23): `Kind::ALL` minus hidden for the
  project (`Preferences::hidden_for`, `preferences.rs:315-324`, merges `hidden_providers` and
  `project_providers[cwd].hidden`) and, once probed, minus not-installed.
- `should_prompt_for_provider(cwd, default_kind, surfaces, available)` (lines 29-37):
  `new_session_surface(default_kind, cwd, preferred).is_tui() && available.len() >= 2`.
  `new_session_surface` (`surface.rs:63-69`) forces Chat for remote projects.
- `Shell::should_prompt_for_provider` (lines 79-92) passes `default_choice().kind` (from
  `new_chat_choice::resolve`) and `provider_surfaces` (empty map when `!surface_supported`).
- Used by `request_new_session` (line 96) and `request_new_session_in_folder` (line 111).
  Tests: `shell/surface_tests.rs:264, 269`.

`new_chat_choice::resolve` (`new_chat_choice.rs:42-73`): project provider -> `default_provider`
-> `recent_models[0]` -> Codex -> first allowed; each step skipped when hidden (`allowed`,
lines 37-40; remote projects ignore hidden).

For "show picker if any enabled agent is TUI": the predicate in `should_prompt_for_provider`
(line 36) would read `available.iter().any(|k| surface(k).is_tui())` instead of the default
kind's surface; `available` already excludes hidden (= disabled) and uninstalled agents, and the
remote check in `new_session_surface` and the `available.len() >= 2` guard are in the same line.
The doc comment and `HINT` copy (lines 8-10, 25-28) describe the old rule. `project_providers`
feeds both `hidden_for` and `new_chat_choice::resolve` step 1, which the PRD says stops applying.

---

## 6. Codex history import

Not found: no code imports Codex conversations into the native store, in `monocode-core`,
`monocode-gpui`, `src/` or `src-tauri/src` (searched `rollout-`, `codex.*import`, `history`,
`import_`). The only import is `Store::import_monocode` (`monocode-core/src/store/monocode_import.rs:46`),
which copies the Tauri app's `monocode.db` sessions.

Code that reads a Codex home today:

| Location | What |
|---|---|
| `accounts.rs:108-124` `Profile::home` | account dir, else `$CODEX_HOME`, else `~/.codex` |
| `accounts/identity.rs:72-79` | reads `<home>/auth.json` for the signed-in identity |
| `accounts/transfer.rs:81` + `accounts/transfer/codex.rs:20-50` | copies a thread's `rollout-*.jsonl[.zst]` from `sessions/` and `archived_sessions/` into another account's `sessions/YYYY/MM/DD/` |
| `provider_cleanup.rs:226-291, 383-..., 503` | deletes rollouts / state for removed sessions across Codex homes (incl. Mono homes) |
| `mcp.rs:436-494` | reads `$CODEX_HOME/config.toml` for MCP discovery |
| `mono/desktop.rs:239` `codex_home(root, account)` | Mono-scoped Codex home used in `provider.rs:1582-1591` |
| `provider.rs:620-626` | resume uses `thread/resume` with `excludeTurns: true` (the app keeps its own transcript; it never pulls Codex history) |

The rollout path parsing/validation in `accounts/transfer/codex.rs:8-50` (`validate_thread_id`,
`rollout_dest`) is the closest existing code to reading Codex history files.

---

## Caveats / not verified

- TUI flags for Grok, Cursor, OpenCode, Hermes, Fx, Devin and `agy` were not checked: those CLIs
  are not installed on this machine. Codex (0.172.0), Claude (2.1.331) and omp `--help` were read
  locally; Pi was not installed.
- Codex 0.172.0 `-a` lists only `on-request` and `never`; the app-server path still sends
  `untrusted` for Supervised (`agent.rs:51-56`). Not tested whether app-server still accepts it.
- Claude 2.1.331 `--permission-mode` help lists `manual`, not `default`; not tested whether
  `default` is still accepted as an alias.
- Remote (`remote://`) chat sessions run through the Host (`runtime.rs:105`), outside every
  injection point above.
- Devin is in `accounts::PROVIDERS` (`accounts.rs:45`) but `supports_profiles` is Claude/Codex
  only (`accounts.rs:216-218`); a non-default Devin profile makes `Profile::directory` return an
  error (`accounts.rs:94-96`) before the `unreachable!()` in `env` (`accounts.rs:157`). Observed
  from code only.
