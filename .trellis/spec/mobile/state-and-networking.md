# Mobile State and Networking

The phone never talks HTTP to a server. Every desktop call goes through the native module's
single `execute(json)` function, which the Rust crate `crates/mobile-link` (`ffi.rs`, enum
`Call` tagged by `operation`) turns into a QUIC exchange with the paired desktop.

## The bridge (`src/features/connection/nativeLink.ts`)

- `nativeCall<T>(input)` stringifies `input`, calls `module.execute`, and expects
  `{ ok, result | error }`; `ok: false` throws `Error(response.error)`.
- Operations used from JS: `identity`, `fingerprint`, `proof` (pairing), `exchange` (one
  request), `watch` / `next` / `unwatch` (streams), plus native transcript operations. Adding an
  operation means adding a `Call` variant in `crates/mobile-link/src/ffi.rs`.
- `request(pairing, method, params, id)` sends
  `{ version: 1, id, method, params }` with the desktop's pinned fingerprint and addresses.
  `version: 1` matches `VERSION: u32 = 1` in `crates/mobile-link/src/lib.rs`.
- Watches: `openWatch(pairing, method, params)` returns a `watchId`;
  `nextUpdates(watchId, timeoutMs)` returns the frames that arrived and throws once the watch
  ended; `closeWatch` never throws.
- `makeCommandId()` builds request IDs; the desktop deduplicates mutations by ID.

## Method names belong to the desktop

The desktop lists every method it answers in `crates/monocode-core/src/mobile/host.rs`:
`HOST_METHODS` (`device.enroll`, `device.leave`, `workspace.list`, `sessions.list`,
`device.push`, `device.selection`, `conversation.watch`) and `APP_METHODS`
(`conversation.read`, `conversation.send`, `session.start`, `session.cancel`, `models.list`,
`project.git`, `mono.update`, ...). Each needs a class in `policy::METHODS`
(`crates/monocode-core/src/mobile/policy.rs`), and phones cannot use full access. A method the
phone calls but the desktop lacks fails at runtime; check both lists before adding a call.
The web client's schemas in `web/protocol/src/methods.ts` are derived from the result shapes
in `mobile/src/features/**`, so changing a shape the phone reads affects that contract too.

## Connection state (`src/features/connection/ConnectionProvider.tsx`)

- Pairing: `parseInvitation(text)` → reuse or create the phone identity (`identity`) → store
  it in SecureStore with `WHEN_UNLOCKED_THIS_DEVICE_ONLY` → `fingerprint` and `proof` →
  `device.enroll` via `exchange`. Private keys live only in SecureStore; `disconnect` keeps the
  identity so a new invitation can recover.
- Catalog: `refresh()` calls `workspace.list`, validates that `projects`, `monos`, `sessions`
  are arrays, saves new desktop `addresses`, and persists the catalog only when its JSON
  changed. It polls every 5 s while `AppState` is `active` and refreshes on returning to the
  foreground.
- Stale results are dropped with `active.current !== value` and a `generation` ref bumped on
  pair and disconnect. Any new async path in a provider must make the same check before
  setting state.
- `withUnchanged`/`sameItems` keep unchanged items' object identity so memoized rows skip
  re-rendering, and the context value is wrapped in `useMemo` because every screen reads it.
  Keep both when adding fields.
- Consumers call `useConnection().command(method, params, id?)`; it throws "Connect your
  desktop first" without a pairing.

## Conversations (`src/features/workspace/WorkspaceProvider.tsx`)

- Opening a conversation reads `conversation.read`, then runs a loop that opens
  `conversation.watch` (`{ conversationId, tail: TAIL, incremental: true, from? }`) and pulls
  frames with `nextUpdates(watchId, 10_000)` while the app is active. On any error it re-reads
  and retries after a pause; when backgrounded it pauses and closes the watch.
- Frames are applied by `applyFrames` in `src/features/workspace/watchUpdates.ts`: whole
  `turns`, `order`, or `patches` with path edits (`value`, `append` at offset `from`,
  `remove`). An edit that does not match the current value throws "needs a fresh snapshot";
  the caller then reopens the watch. Never apply a partial patch.
- Mutations go through `mutate(method, params)`: one command ID per `[method, params]`
  signature, kept after a failure so a user retry after a lost reply reuses it (the desktop
  deduplicates), at most 128 kept.
- Latest turns of the 12 most recent conversations are cached in kv-store
  (`monocode.mobile.<desktop>.turns.<id>` plus an index); local `pending:` turns are never
  cached.
- Commands are not queued offline; failed-send drafts are kept (`mobile/README.md`).

## Compatibility with older desktops

Desktops update independently. New result fields are optional and gate UI:
`Catalog.monoTools`, `push`, `monoFiles` (read by `useMonoTools()` / `useMonoFiles()`),
`Conversation.turns?` ("Missing from desktops that predate turn-shaped transcripts"),
`question?`, `usageLimit?`. Add new desktop features the same way: an optional field or flag,
with the phone hiding the feature when it is absent.

## Storage

- `expo-sqlite/kv-store` for non-secret state, `expo-secure-store` for the identity only.
- Keys: `monocode.mobile.pairing`, `monocode.mobile.identity`,
  `monocode.mobile.<fingerprint>.selection|catalog`, `monocode.mobile.<desktop>.turns*`,
  `monocode.lastPlace.<desktop>`, `monocode.monoSeen.<desktop>`, `monocode.monoAccents`,
  `monocode.reactions.<id>`, and unprefixed settings such as `toolCalls.collapsed`.
  Renaming a key loses that data on installed phones; treat them as identifiers, like the
  `monocode.*` prefix in root `CLAUDE.md` "Never rename these".
- Scope per-desktop data by the desktop fingerprint, as the existing keys do.
- Wrap storage reads in `try/catch` and fall back to defaults (`load()` in
  `src/features/settings/toolCalls.ts`, `readCachedTurns`).
