# Host (`host/`) Guidelines

The remote Host is the headless Node process that runs agent providers on another machine
(Windows, Linux or macOS) for the desktop apps. It ships as the `monocode-host` CLI, keeps its
own SQLite session database, and answers one authenticated JSON-RPC endpoint on loopback
(`127.0.0.1:3774` by default). User-facing behavior is described in
[`docs/remote-access.md`](../../../docs/remote-access.md).

## What this layer owns

- The CLI and process lifecycle: `host/cli.ts` (`serve`, `start`, `service install|uninstall`,
  `status`, `stop`, `pair`, `devices`, `revoke`, `connection-info`, `--version`).
- The RPC server and its method table: `host/server.ts` (`createHostServer`).
- Session execution and durability: `host/engine.ts` (`HostEngine`), `host/store.ts` (`HostStore`).
- Provider discovery and launch: `host/process.ts`, `host/child-backend.ts`, `host/providers.ts`.
- Workspace, Git, uploads, kit and session moves (see the file table below).
- The fork's Host version: `host/VERSION` (`X.Y.Z+monoptah`), exposed by `host/host-version.ts`.

The Host is **not** self-contained: it bundles shared TypeScript from `src/` (provider adapters
under `src/integrations/harness/`, session model under `src/features/sessions/model/`, and the
wire types in `src/features/connections/model/protocol.ts`). A change there changes the Host.

## Pre-Development Checklist

1. Read [protocol-and-compat.md](./protocol-and-compat.md) before touching `server.ts`,
   `engine.ts` (`parseCommand`), `store.ts` or `src/features/connections/model/protocol.ts`.
2. Read [structure-and-processes.md](./structure-and-processes.md) before touching the CLI,
   service install, owner lock, provider resolution or child processes.
3. Read [testing.md](./testing.md) before adding or changing tests.
4. Re-read root `CLAUDE.md` "Never rename these": the `monocode-host` CLI name, the
   `~/.monocode-host` data dir and the `com.monocode.host` launchd label stay as they are.
5. If the change is user-visible on installed Hosts, plan a Host release (root `CLAUDE.md`,
   "Releasing the Host"): bump `host/VERSION`, tag `host-vX.Y.Z`, then pin it in the app with
   `scripts/pin-host-release.mjs` (writes `crates/monocode-core/src/remote/host_release.rs`).
   Never re-upload an existing Host version: the app pins each asset's sha256.

## Quality Check

```bash
npm run host:build      # tsc --noEmit -p host/tsconfig.json && node host/build.mjs
npm run test:host       # pretest builds the bundle, then vitest --config host/vitest.config.ts
node --test host/runtime-download.test.mjs
./scripts/check-fork-branding.sh   # host/ is scanned; host/windows.ts is allow-listed
```

Read exit codes directly, never through a pipe (root `CLAUDE.md`, "Verifying").
`host/tsconfig.json` excludes `*.test.ts`, so `host:build` does not type-check tests; vitest
only transpiles them. The Tauri `ci.yml` job that runs `test:host` is paused
(`workflow_dispatch` only) and `native.yml` does not run Host tests, so **nothing in PR CI runs
the Host suite**: run it locally for any `host/` or shared `src/` change the Host bundles.

## Files

| File | Role |
|---|---|
| `cli.ts` | Entry point bundled to `build/host/monocode-host.mjs`; commands, `running.json`, `/lifecycle` admin route |
| `server.ts` | `createHostServer`: auth, `/rpc` dispatch, `environment.describe` capabilities, model catalog cache |
| `engine.ts` | `HostEngine`, `parseCommand`: commands, receipts, live sessions, 120 ms batching, restart recovery |
| `store.ts` | `HostStore`: SQLite schema, snapshots, event journal, receipts, device credentials |
| `owner.ts` | `acquireHostOwner`: one Host per data dir (SQLite exclusive lock + `owner.lock` v2) |
| `host-version.ts` | `HOST_VERSION`, `HOST_FLAVOR = "monoptah"`, `HOST_VERSION_BASE` from `__HOST_VERSION__` |
| `providers.ts` | `HostProvider` interface and `hostProviders` table mapping to `src/integrations/harness` adapters |
| `process.ts` | `resolveProvider`, `providerLaunch`: finds provider CLIs on PATH and known install dirs |
| `child-backend.ts` | `HostChildBackend`: Node implementation of the harness `ChildBackend` (spawn, exec, OpenCode HTTP) |
| `provider-guard.mjs` | Copied next to the bundle; kills the provider when the Host's fd 3 closes |
| `service.ts` | `installService`/`uninstallService`/`connectionInfo`: launchd, systemd user unit, detached start |
| `windows.ts`, `windows-acl.ps1` | PowerShell helpers, directory ACLs, Task Scheduler `MonoCode Host-<SID>` |
| `workspace.ts` | `workspacePath`/`existingPath` guards, file read/write/search, Git index/diff/actions |
| `workspace-commands.ts` | `WorkspaceCommands`: answers the app's Tauri file command names (`WORKSPACE_COMMANDS`) |
| `workspace-watch.ts` | File and ref watchers behind the `workspace.watch` capability |
| `browse.ts` | `projects.browse`: lists directories without reading files |
| `git-branches.ts`, `git-worktrees.ts`, `git-blame.ts` | Branch list/switch/create, worktrees, blame parsing |
| `attachments.ts` | Session attachments (20 MiB, 512 KiB chunks, offset-idempotent) |
| `uploads.ts` | `HostUploads`: staged, sha256-verified uploads with quota and sweeper |
| `sync-transfer.ts` | `SyncTransfers`: splits syncs above 4 MiB into chunks under the 16 MiB response cap |
| `kit.ts` | `HostKit`: Claude skills/plugins/MCP kit, `scrubSecrets`, `installLogScrubber` |
| `move.ts`, `transcript-sync.ts` | `HostMoves`: `workspace.applySnapshot`, `sessions.import`; TS mirror of Rust transcript sync |
| `build.mjs` | esbuild bundle (node24, ESM), injects `__HOST_VERSION__`, copies `provider-guard.mjs` |
| `package.mjs`, `runtime-download.mjs` | Self-contained archives with pinned, checksummed Node runtimes |
| `vitest.config.ts`, `tsconfig.json`, `VERSION` | Test config, compiler config, fork Host version |

## Topic Files

| Guide | Covers |
|---|---|
| [protocol-and-compat.md](./protocol-and-compat.md) | RPC envelope, capabilities, idempotent commands, what installed Hosts and apps depend on |
| [structure-and-processes.md](./structure-and-processes.md) | Module layering, lifecycle, ownership lock, provider processes, path and secret safety |
| [testing.md](./testing.md) | Vitest setup, fixtures, fake providers, platform gating, what needs the built bundle |
