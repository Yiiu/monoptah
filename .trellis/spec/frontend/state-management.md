# State Management

No state library. State lives in four places, each with a clear job:

| What | Where | Mechanism |
|---|---|---|
| Workspace UI state (open views, tabs, selected project, dialogs) | `src/app/App.tsx` | ~80 `useState` hooks, passed down as props |
| Sessions (transcripts, streaming text) | `SessionStateStore` in `src/features/sessions/model/sessionStateStore.ts` | External store + `useSyncExternalStore` |
| Durable data (sessions, projects, git, mail, notes, ...) | Rust, via `invoke` | e.g. `upsertSession` in `src/features/sessions/data/sessionStore.ts` |
| Preferences and small UI memory | `localStorage`, keys prefixed `monocode.` | load/save helpers in each feature's `model/` |

React contexts are few (10 non-test files) and carry handles, not data: `SessionStateContext`,
`NativePopupHost` (`src/shared/ui/NativePopupHost.tsx`), `InDialog`
(`src/shared/ui/Popover.tsx`), `RevealedSetting` (`src/features/settings/ui/primitives.tsx`).

## Sessions: `SessionStateStore`

`src/features/sessions/model/sessionStateStore.ts`:

- One authoritative `Session[]` snapshot plus a `byId` map and a `ref` for synchronous readers.
- Three listener sets: `subscribeShell` (list-level UI), `subscribe(id, ...)` (one session),
  `observe` (everything, e.g. persistence).
- `update(next, textOnly)` notifies per-id listeners only for sessions whose object identity
  changed, and skips shell listeners for text-only streaming updates. Therefore **always
  update immutably**: replace the changed `Session` object, keep unchanged ones identical.
- `useWorkspaceSessionState(initial)` (used by `App.tsx`) returns `{ sessions, setSessions,
  sessionsRef, store }`. `setSessions` has the `Dispatch<SetStateAction<Session[]>>` shape.
- Per-session components use `useSessionSnapshot(fallback, visible)`; hidden panes keep their
  last snapshot and do not subscribe.

Persistence of sessions goes to Rust, not localStorage: `upsertSession` serialises writes per
session through `enqueueSessionWrite` and sends `session_upsert` or, for Mono sessions,
`mono_session_upsert` with only the changed block tail. Restored data is sanitised
(`sanitizeSessionForPersist`, `sanitizeLinkedWorkItem`, `HARNESSES.includes(...)` checks)
before use; see type-safety.md.

## localStorage preferences

**Never rename an existing `monocode.*` key** (repo `CLAUDE.md`, "Never rename these"): it
silently drops every saved preference on existing installs. New keys also use the
`monocode.` prefix for consistency (`src/features/mail/model/mailPrefs.ts` documents this).
About 130 distinct keys exist; grep `"monocode.` before inventing one.

Rules seen in every helper:

- Wrap every `localStorage` access in `try/catch` (private mode, quota) and fall back to a
  default. Examples: `loadFollowUpBehavior` / `saveFollowUpBehavior` in
  `src/features/settings/model/settings.ts`, `readFlag` / `writeFlag` in
  `src/features/settings/model/storageFlags.ts` (booleans stored as `"1"`/`"0"`, read
  `"1"` or `"true"`).
- Validate what you read: enums are checked against their allowed values, numbers with
  `Number.isInteger` and bounds, JSON with `isRecord`-style guards (type-safety.md).
- Define the key once as a constant (`const FILE_TAB_MODE_KEY = "monocode.fileTabMode"`, or a
  `MAIL_PREF_KEYS` object `as const`) and export a `XXX_DEFAULT`.
- When the stored shape changes incompatibly, add a new versioned key rather than reinterpret
  the old one (`monocode.notificationProjects.v2` in
  `src/features/notifications/model/notificationProjects.ts`; `.v1` suffixes elsewhere), and
  read old formats tolerantly where possible (`parseExcludedMembers` in
  `src/features/ticket-contributors/model/prefs.ts` still accepts bare ids).

### Reactive preferences: `prefStore`

When several components or windows must react to a preference, follow `prefStore` in
`src/features/mail/model/mailPrefs.ts`. It returns `{ load, save, subscribe, useValue, event }`:

- `save` writes, then dispatches a window `CustomEvent` named from the key
  (`monocode:mail-<name>-change`), so same-window subscribers update.
- `subscribe` also listens to the `storage` event, which reports writes from other windows
  (quick composer, floating Mono, transferred windows share localStorage).
- `load` caches the raw string and parsed value so `useSyncExternalStore` gets a stable
  reference; if storage refuses the write, an in-memory `unsaved` value keeps the control
  working for this window.
- Typed builders: `enumPref`, `nullableString`, `positiveInt`.

## Cross-component notifications

Many modules broadcast changes with `window.dispatchEvent(new CustomEvent(...))` using names
of the form `monocode:<thing>-change` / `monocode:<thing>-changed` (about 100 names, e.g.
`monocode:mono-files-changed`; `subscribeGitChanged` in `src/platform/tauri/fs.ts` wraps one).
Export a `subscribeXxx(listener) => unsubscribe` helper next to the dispatcher instead of
spelling the event name in consumers. These names are internal identifiers, not branding.

## What goes where (decision guide)

- Survives restart and is real data (sessions, notes, mail, project config): Rust command.
- Survives restart and is a user preference or "dismissed" flag: `monocode.*` localStorage
  key with load/save helpers in the feature's `model/`.
- Shared by many components and fetched from Rust: module-level store + hook
  (hook-guidelines.md, Pattern 1).
- Owned by one view: `useState` in that component. Workspace-wide: `App.tsx`.
