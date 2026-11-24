# Platform Bridge (Tauri commands and events)

The frontend talks to Rust (`src-tauri/src/`) through `@tauri-apps/api` (`invoke`,
`listen`, `getCurrentWebviewWindow`) and plugins (`@tauri-apps/plugin-dialog`, `-opener`,
`-process`, `-updater`). There is no generated binding layer: every command is a hand-typed
wrapper.

## Calling a command

Rust commands are registered in `tauri::generate_handler![...]` in `src-tauri/src/lib.rs`
(around line 339). A frontend wrapper is a small exported function with explicit types:

```ts
// src/platform/tauri/fs.ts
export function createPath(parent: string, name: string, isDir: boolean): Promise<string> {
  return invoke<string>("create_path", { parent, name, isDir }).then(slash);
}
```

- Command names are `snake_case` strings; argument keys are **camelCase** and Tauri maps them
  to the Rust snake_case parameters (`isDir` -> `is_dir: bool` in `src-tauri/src/fs.rs`
  `create_path`). A misspelled key fails only at runtime.
- Always pass the result type: `invoke<SessionSummary[]>("session_list_by_project", ...)`.
  Rust `Option<T>` maps to `T | null`; send `null` explicitly for optional args
  (`identity: identity ?? null` in `resolveProjectLocation`).
- Rust `Result<_, String>` rejects the promise with a string, not an `Error`. Format with a
  helper like `errorText(cause)` in `src/features/telegram/ui/LinkTelegramTopicDialog.tsx`
  (`cause instanceof Error ? cause.message : String(cause)`).
- Best-effort calls end with `.catch(() => undefined)` (quit protocol in
  `src/app/model/appLifecycle.ts`); unawaited calls are prefixed with `void`.
- Put wrappers used across features in `src/platform/tauri/fs.ts` (91 exports) or a sibling
  (`pty.ts`, `clipboard.ts`); feature-only commands go in the feature's `model/` (e.g.
  `telegramStatus` in `src/features/telegram/model/telegram.ts`) or `data/`
  (`src/features/sessions/data/sessionStore.ts`).
- Guard native-only calls with `isTauri()` and platform flags from
  `src/platform/tauri/platform.ts` (`IS_MAC`, `IS_WIN`, `IS_LINUX`, `MOD`), as
  `src/app/hooks/useFloatingMono.ts` does.

Adding a command is a cross-layer change: Rust command + registration + wrapper + test with
a mocked `invoke`. Read `../backend/` and `../guides/cross-layer-thinking-guide.md`.

## Remote paths: `invokeWorkspace`

`src/platform/tauri/fs.ts` aliases `invoke` to `invokeWorkspace`. When any of the args
`path`, `cwd`, `parent`, `from`, `destParent`, `paths` (or `options.cwd`) starts with
`REMOTE_PATH_PREFIX` (`remote://`, `src/shared/lib/remotePaths.ts`), the call goes to the
runner installed by `setRemoteCommandRunner` (from
`src/features/connections/model/remoteCommands.ts`, imported for its side effect in
`src/main.tsx`) instead of local Rust. File and git commands must therefore go through these
wrappers and use those argument names so remote projects work.

## Events

- Global events: `listen<Payload>("pty-data", handler)` returns `Promise<UnlistenFn>`. Menu
  commands (`toggle_sidebar`, `split_right`, `next_tab`, ...) are listened in `App.tsx`;
  their cleanup is `void Promise.all(unlisten).then((fns) => fns.forEach((fn) => fn()))`.
- **Window-scoped events** use `getCurrentWebviewWindow().listen(...)`. A global `listen`
  matches events for every target, so a per-window prompt would appear once per window (the
  comment above `quit_confirm` in `src/main.tsx`).
- High-volume streams use one shared, ref-counted bridge: `src/platform/tauri/pty.ts` installs
  `pty-data`/`pty-exit` listeners once (`users` count, delayed teardown via `teardownTimer`),
  routes by id through handler maps, buffers output for unmounted views within
  `MAX_BUFFERED_BYTES`, and decodes base64 defensively (`decodePtyChunk` returns `null`
  instead of throwing). Harness output (`harness-stdout`, `harness-stderr`, `harness-exit`,
  `harness-sse`) is bridged the same way in `src/integrations/harness/core/child.ts`.
- Harness code must use the `invoke`/`listen` functions in `core/child.ts`, which delegate to
  a `ChildBackend` when `configureChildBackend` was called (the headless Host does this), else
  to Tauri. Do not import `@tauri-apps/api` directly in new harness protocol code.

## Mocking in tests

Every test touching a wrapper mocks the Tauri module (about 145 test files):

```ts
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
```

- Route by command name when a component makes several calls:
  `invoke: vi.fn(async (command: string) => command === "list_external_editors" ? [] : null)`.
- Add what the code under test imports from the module: `isTauri: () => true`,
  `convertFileSrc: (path: string) => path`.
- Window events: mock `@tauri-apps/api/webviewWindow` with
  `getCurrentWebviewWindow: () => ({ listen: tauri.listen })`.
- Assert on calls: `expect(invoke).toHaveBeenCalledWith("create_path", { parent, name, isDir })`
  pins the exact argument keys the Rust side expects.
- Browser tests (`tests/browser/`) run without Tauri. In `benchmark*` Vite modes,
  `vite.config.ts` aliases `@tauri-apps/api/core` to `tests/browser/tauriFixture.ts`, whose
  `invoke` uses `window.workspaceInvoke` when a page installs it.
