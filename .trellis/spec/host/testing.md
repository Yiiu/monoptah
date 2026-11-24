# Host Testing

## How the suite runs

- `npm run test:host` = `vitest run --config host/vitest.config.ts`, preceded by the
  `pretest:host` hook (`npm run host:build`). The build matters: `host/cli.test.ts` executes
  `build/host/monocode-host.mjs` and `host/child-backend.test.ts` executes
  `build/host/provider-guard.mjs`. When running a single file directly with
  `npx vitest run --config host/vitest.config.ts host/<file>.test.ts`, run
  `npm run host:build` first if that file uses the bundle.
- `host/vitest.config.ts`: `environment: "node"`, `include: ["host/**/*.test.ts"]`,
  `fileParallelism: false`, `testTimeout`/`hookTimeout` 30 s, and it defines
  `__HOST_VERSION__` from `host/VERSION` exactly as `host/build.mjs` does. Files run serially
  because they launch real Git, Node and PowerShell processes; do not re-enable parallelism.
- `node --test host/runtime-download.test.mjs` covers the packaging downloader separately.
- Tests are colocated as `host/<module>.test.ts` (plus scenario files such as
  `move.resume.test.ts`, `move.server.test.ts`, `uploads.shortwrite.test.ts`,
  `git-idle-guard.test.ts`, `large-sync.test.ts`). `host/tsconfig.json` excludes them, so type
  errors in tests are not caught by `host:build`.
- No PR CI job runs this suite today (Tauri `ci.yml` is `workflow_dispatch`-only); the local run
  is the gate.

## Fixtures used across files

**Temporary data dir per test, cleaned in `afterEach`.** Every suite creates
`mkdtempSync(join(tmpdir(), "monocode-<area>-test-"))`, opens a real `HostStore` on
`host.db` inside it, and registers a cleanup that closes the engine and store and removes the
directory (`setup()` in `host/engine.test.ts` and `host/server.test.ts`; cleanups run in
reverse order in `engine.test.ts`).

**In-process fake provider.** Engine and server tests pass a `HostProvider` whose `send`
returns a promise the test resolves, so turns can be held "running":

```ts
send: vi.fn((input) => new Promise<void>((resolve) => {
  turns.push({ input, finish: resolve });
})),
cancel: vi.fn(async () => { turns.at(-1)?.finish(); }),
```

**Real server on an ephemeral port.** `server.test.ts` calls `createHostServer(engine,
providers, undefined, { kitHome })`, listens on `127.0.0.1:0`, issues two devices with
`store.issueDevice`, and posts to `/rpc` through a `call(method, params, token, overrides,
headers)` helper. Use it for any new RPC method: test auth, `environmentId`, the happy path and
rejected input in the same file. Open projects with `engine.openProject(directory)` so paths
are canonicalized as in production (Windows 8.3 names like `RUNNER~1`).

**Fake provider executables.** Transport tests write a Node script that speaks the provider's
wire format and point `HostChildBackend` at it (`new HostChildBackend({ codex: file })` and
`configureChildBackend(...)`; see `host/provider-transport.test.ts`, which also mocks
`node:os` `homedir` to a fixture home). OpenCode tests start a local `http.createServer` with
SSE (`host/opencode-transport.test.ts`). No test may contact a real provider or paid model.

**Mocked catalogs.** Model discovery is replaced with `vi.mock(".../codexCatalog", ...)` and a
hoisted `vi.fn()` (`server.test.ts` `modelProbe`) to test cache invalidation (binary path or
mtime changes, five-minute expiry).

## Platform gating

The Host supports Windows, Linux and macOS, and tests run on Windows in this checkout. Gate
platform-specific tests rather than weakening them:

- `it.skipIf(process.platform === "win32")` (`bootstrap.test.ts`, `service.test.ts`),
  `it.runIf(process.platform !== "win32")` (`process.test.ts`),
  `it.runIf(process.platform === "linux")` for systemd, and
  `const posix = process.platform === "win32" ? it.skip : it` (`kit.test.ts`).
- Windows behavior is tested by passing the platform explicitly where the code accepts it,
  e.g. `providerLaunch(file, args, "win32")` in `windows.test.ts`, and by inspecting generated
  scripts (`windowsTaskScript`) rather than registering real scheduled tasks.
- Use longer timeouts on Windows where processes start slowly (`cli.test.ts` uses 25 s vs 10 s
  per command and 60 s vs 20 s per test). Symlink tests use `"junction"` on Windows
  (`server.test.ts`).
- Tests never install or replace a real user service; `service.test.ts` checks generated unit
  and plist text and mocked registration.

## What to assert

- Protocol changes: describe output (`"identifies itself as the Monoptah flavor in
  describe.host"`, capability lists), revoked-device and browser-origin rejection, and
  identity change errors, all in `server.test.ts`.
- Command idempotency and restart recovery belong in `engine.test.ts` (same `commandId`
  returns the same receipt; different payload throws; `running` becomes `interrupted` on a new
  `HostEngine`).
- Path handling: traversal, `.git` segments and symlink or junction escapes.
- A test that still passes only because it asserts "MonoCode" copy is a failure (root
  `CLAUDE.md`); user-facing Host text says "Monoptah Host".
