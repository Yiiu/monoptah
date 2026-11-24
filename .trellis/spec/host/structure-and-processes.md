# Host Structure and Processes

## Layering

```
host/cli.ts            parses argv, owns data dir, running.json, lifecycle
  └─ host/server.ts    HTTP + /rpc switch; thin: validates params, calls modules
       ├─ host/engine.ts   HostEngine: sessions, commands, provider dispatch
       │    └─ host/store.ts  HostStore: the only SQLite writer
       ├─ workspace*.ts, git-*.ts, browse.ts, uploads.ts, attachments.ts, kit.ts, move.ts
       └─ ../src/integrations/harness/**   provider adapters, shared with the desktop
host/child-backend.ts  HostChildBackend: the harness ChildBackend for Node (spawn/exec/HTTP)
host/process.ts        resolveProvider / providerLaunch
```

- `server.ts` keeps one `case "<method>"` per RPC and delegates. Put logic in a module with its
  own test file, not in the switch. Parameters arrive as `Record<string, unknown>`; coerce and
  validate each one (`String(params.x ?? "")`, `Number.isSafeInteger(...)`) as existing cases do.
- Provider behavior lives in `src/integrations/harness/providers/*`. The Host wires them through
  `hostProviders` (`host/providers.ts`) and the `HostProvider` interface (`send`, `cancel`,
  `stop`, `bind`, `approve`, `answer`, optional `compact`, `generateTitle`,
  `generateBranchName`). Do not fork provider logic into `host/`.
- Harness code reaches processes only through `configureChildBackend(backend)`
  (`src/integrations/harness/core/child`). `cli.ts` installs `HostChildBackend`; tests install
  one with fake binaries.
- Git or checkout-changing work that must not race a running turn goes through
  `HostEngine.withIdleProject(projectId, action, what)`; `WorkspaceCommands` receives it as a
  callback. It refuses while any session in the project is `running` or another action holds it.

## Lifecycle (`host/cli.ts`)

- `serve` acquires ownership, resolves which `REMOTE_PROVIDERS` exist, builds
  `HostEngine(store, hostProviders)`, listens on `127.0.0.1` only, then writes
  `running.json` (`{ pid, port, secret }`, mode 0600). Shutdown (`SIGTERM`, `SIGINT`, or
  `/lifecycle` `stop`) closes the server, `engine.close()`, `backend.close()`, then removes
  `running.json` and the owner lock.
- `start` spawns a detached `serve` logging to `host.log` and waits for `running.json` with its
  child's PID. `service install` delegates to `installService` (`host/service.ts`): launchd
  `com.monocode.host`, systemd user unit `monocode-host.service` (with `loginctl enable-linger`),
  detached process when `systemdBooted()` is false, Task Scheduler on Windows
  (`windowsTaskScript` in `host/windows.ts`). Installing must not restart a running Host
  (asserted in `host/cli.test.ts`).
- `service uninstall` removes the service and stops the Host but never deletes the data dir.
- `process.umask(0o077)`, `chmodSync(directory, 0o700)` on Unix and
  `protectWindowsDirectory` on Windows: new files and dirs must stay owner-only; pass
  `mode: 0o600` when writing secrets (see `running.json`, `owner.lock`).

## One Host per data directory (`host/owner.ts`)

`acquireHostOwner` takes `BEGIN EXCLUSIVE` on `owner.db`; failure means "A host already owns
this data directory". It then inspects a legacy `owner.lock`: a PID file from an old Host is
honored if that PID is alive and its command matches `/monocode-host.*\bserve\b/`, or when the
command cannot be read. It writes `{ version: 2, pid }` so older Hosts never treat the file as a
dead PID. Never treat an unresponsive Host as dead.

## Provider processes

- `resolveProvider` (`host/process.ts`) searches PATH plus per-provider install dirs
  (`providerDirectories`) and verifies ambiguous names (`cursor` → `agent`, Pi markers).
  `providerLaunch` runs `.js/.mjs/.cjs` entries with the bundled Node and, on Windows, maps
  npm `.cmd` wrappers to their package entry (`npmEntries`); other `.cmd/.bat` are rejected,
  never run through a shell.
- `cli.ts` prepends `dirname(process.execPath)` to `PATH` so npm-based providers find the
  bundled Node.
- `HostChildBackend.invoke("harness_spawn")` launches every provider through
  `provider-guard.mjs` with a 4th pipe; the guard kills the provider when the Host dies. It
  sets `MONOCODE_HOST=1`, `detached` on Unix, `windowsHide`. Stopping sends `SIGTERM`, then
  `SIGKILL` after 3 s; Windows uses `taskkill /T /F`.
- `harness_exec` only allows argument lists from `ALLOWED_EXEC_ARGS` (plus OpenCode, Devin and
  Grok specific lists). A new catalog or status probe must be added there explicitly.
- OpenCode HTTP is limited to loopback by `loopbackUrl`. Named provider accounts are rejected
  ("not supported by this host yet").

## Safety rules that every module follows

- **Paths:** take client paths only through `workspacePath` / `existingPath`
  (`host/workspace.ts`). They reject `..`, absolute escapes, NUL, over-4096 chars and any
  `.git` segment, and `existingPath` re-checks after `realpath` (symlink and junction escapes,
  tested in `server.test.ts` "reads host files while rejecting traversal and symlink escapes").
  `openProject` accepts only absolute directories and stores the `realpath`.
- **Secrets:** errors returned to clients go through `scrubSecrets`; `installLogScrubber`
  patches the console while the server runs. Kit secrets must never reach logs.
- **Durability:** session writes go through `HostStore.transaction` (`BEGIN IMMEDIATE`, cache
  cleared on rollback). `HostStore` is the only writer and serves cached snapshots that callers
  must treat as immutable. Streamed events are batched for `FLUSH_MS` (120 ms); approvals,
  questions, errors and completion are written immediately (`BATCHED` set in `engine.ts`).
- **Platform:** Windows code paths use `runPowerShell`/`psQuote` from `host/windows.ts`
  (Windows PowerShell 5.1); do not assume a Unix shell exists on Windows hosts.

## Building and packaging

`host/build.mjs` bundles `host/cli.ts` with esbuild (`platform: "node"`, `target: "node24"`,
ESM, `.ps1` loaded as text) to `build/host/monocode-host.mjs`, defines `__HOST_VERSION__` from
`host/VERSION`, and copies `provider-guard.mjs` beside it. `host/package.mjs` adds pinned,
checksummed Node runtimes (`runtime-download.mjs`); asset names keep `monocode-host-<target>`.
Release flow and tags: root `CLAUDE.md`, "Releasing the Host", and
`scripts/host-release-monoptah.mjs` (plain `X.Y.Z` only; never pushes).
