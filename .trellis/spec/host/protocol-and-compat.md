# Host Protocol and Compatibility

The Host talks to two families of clients it cannot update in lockstep: the Tauri app
(TypeScript, `src/features/connections/`) and the native app (Rust,
`crates/monocode-core/src/remote.rs`, `remote/protocol.rs`, `remote/host_info.rs`). Installed
Hosts also outlive app releases, and the app may connect to an **upstream** MonoCode Host.
Every change here must keep old clients and old Hosts working.

## Wire envelope (`host/server.ts`, `createHostServer`)

- Only `POST /rpc` with no `Origin` header is served; anything else is `403`. There is no CORS:
  the endpoint is meant for native HTTP, never for a browser.
- `Authorization: Bearer <43-char base64url>` must match a device in `HostStore.authenticated`.
  Authentication is checked **again after the body is read** (a device may be revoked while
  the body arrives; covered by `server.test.ts` "rejects a credential revoked while its request
  body is arriving").
- Body: `{ version, environmentId, method, params }`, at most `MAX_BODY` (16 MiB).
  `version` must equal `HOST_PROTOCOL_VERSION` (`1`, in
  `src/features/connections/model/protocol.ts`; mirrored as `HOST_PROTOCOL_VERSION: u64 = 1` in
  `crates/monocode-core/src/remote/protocol.rs`, and sent by `remote.rs` line ~557).
- Every method except `environment.describe` must carry the store's `environmentId`, or the Host
  answers "Host identity changed; reconnect this machine explicitly".
- Success is `{ result }`; failure is HTTP 400 `{ error }`, with the message passed through
  `scrubSecrets` from `host/kit.ts`. Unknown methods throw "Unsupported host method".
- `/lifecycle` is a separate admin route used only by `host/cli.ts` (`status`/`stop`), with a
  per-run secret from `running.json`; it is never sent to a desktop.

## Capabilities, flavor and version

`environment.describe` returns `protocolVersion`, `environmentId`, `name`, `platform`,
`host: { flavor: HOST_FLAVOR, version: HOST_VERSION_BASE }`, `providers` and `capabilities`.

- **Clients gate features on capability strings, never on version numbers** (rule D3 in
  `crates/monocode-core/src/remote/host_info.rs`; e.g. `host.supports("sessions.plan")` in
  `crates/monocode-gpui/src/conversation/remote.rs`). When you add an RPC method or change
  behavior a client must detect, add a new capability string to the `capabilities` array.
  Do not remove or rename an existing one: shipped apps look for it.
- `host` is fork-local. A descriptor without it is read as flavor `upstream`
  (`HostInfo::from_descriptor`). Keep `HOST_FLAVOR = "monoptah"` in `host/host-version.ts`.
- `providers` is filtered: older clients validate the list against Codex and Claude only, so
  newer providers are reported only when the request lists them in
  `params.supportedProviders` (tested by "advertises newer providers only to desktops that
  request them"). Keep that fallback.
- `requireHostDescriptor` in `protocol.ts` rejects a descriptor whose providers are not all in
  `REMOTE_PROVIDERS`; adding a provider means updating `REMOTE_PROVIDERS`, `hostProviders`
  (`host/providers.ts`), `resolveBinary` and `discoverModels` (`host/server.ts`) together.

## Commands are idempotent (`host/engine.ts`, `HostEngine.command`)

Session mutations go through `commands.dispatch` → `HostEngine.command(raw)`:

```ts
const command = parseCommand(raw);
const signature = createHash("sha256").update(JSON.stringify(command)).digest("hex");
const previous = this.store.receipt(command.commandId, signature);
if (previous) return previous;
```

- `parseCommand` validates every field (length caps, no `\0`, `RUNTIME_MODES`, `mc/xxxxxxxx`
  auto-worktree branch). New fields must be validated there, not trusted.
- A reused `commandId` with a different payload throws "Command ID was already used with a
  different payload" (`HostStore.receipt`). Clients retry lost responses with the same ID.
- A receipt (`{ commandId, sessionId, revision }`) means **durable acceptance**, not provider
  completion. The provider effect runs after the transaction commits.
- Never replay a provider send automatically: on startup, sessions still `running` are marked
  `interrupted` ("Host restarted. This turn was interrupted…") in the `HostEngine` constructor.

## Sync, events and size caps

- Each saved snapshot bumps `revision` and appends one row to `events`; `HostStore.save` keeps
  only the last 2 000 revisions per session. `events.read` returns a full `snapshot` when the
  cursor fell out of the journal.
- `sessions.sync` returns only changed blocks (`blockRevisions`); a result over
  `INLINE_SYNC_BYTES` (4 MiB) is served as `sessions.syncChunk` pieces of one revision by
  `SyncTransfers` (`host/sync-transfer.ts`), because clients reject responses over 16 MiB.
- File reads are capped (1 MiB text; `MAX_READ_CHUNK` for `files.readBytes`), attachments at
  20 MiB (`MAX_REMOTE_ATTACHMENT_BYTES`), uploads at 500 MiB with a 2 GiB quota
  (`host/uploads.ts`). Any new method that can return large data must stay under 16 MiB.

## Persistent state that installed Hosts already have

- `HostStore` creates tables with `CREATE TABLE IF NOT EXISTS` and migrates by probing
  (`PRAGMA table_info(sessions)` then `ALTER TABLE sessions ADD COLUMN summary`). Follow that
  additive pattern; never drop or rename a column, and tolerate old snapshots (see `createdAt`
  fallback in `HostStore.save`).
- `environmentId` is created once in `metadata`; clients pin it. Do not regenerate it.
- Device tokens are stored only as sha256 hashes (`HostStore.hash`); `pair` prints the token once.
- `owner.lock` keeps `{ version: 2, pid }` so pre-lock Hosts do not take over (see
  [structure-and-processes.md](./structure-and-processes.md)).

## Identifiers that must not change

From root `CLAUDE.md` and the code: the `monocode-host` CLI and asset names, `~/.monocode-host`
(`cli.ts` `--data-dir` default; also hard-coded in `child-backend.ts` `devinAskEditsArgs`),
`com.monocode.host` (`service.ts` `LABEL`), `monocode-host.service`, and the Windows task name
`MonoCode Host-$sid` (`windows.ts`, allow-listed in `scripts/check-fork-branding.sh`). Text a
user reads, such as `--help` and the `listening on` log line, says "Monoptah Host".
