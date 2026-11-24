# Hook Guidelines

There is no data-fetching library (no React Query, SWR, Redux or Zustand in `package.json`).
Hooks are hand-written on top of `useSyncExternalStore`, `useEffect` and `invoke` wrappers.

## Naming and placement

- `useXxx` in a file of the same name: `src/features/<feature>/hooks/useXxx.ts`, test beside it
  as `useXxx.test.ts` (e.g. `src/features/telegram/hooks/useTelegramLinkable.ts`).
- App-wide hooks that coordinate several features: `src/app/hooks/`
  (`useWorkspaceNavigation.ts`, `useIdleSessionDetach.ts`).
- Feature-free hooks: `src/shared/hooks/` (`useStableValue`, `useDragResize`, `useSortable`,
  `useAnimatedReorder`, `useLockOverscroll`, `useColorScheme`).
- A hook can live in `ui/` when it only serves components there
  (`src/features/sessions/ui/useToolDetail.ts`, `src/app/shell/useProjectMenu.tsx`).
- Export a `useXxxState` variant when callers must distinguish "loading" from "empty", as
  `useProjectBranchesState` (returns `{ branches, settled }`) beside `useProjectBranches` in
  `src/features/source-control/hooks/useProjectBranches.ts`.

## Pattern 1: shared, deduplicated data (module store + `useSyncExternalStore`)

Use this when several components need the same backend data. Reference:
`src/features/source-control/hooks/useProjectBranches.ts`.

- A module-level `Map<key, Entry>` holds `{ state, listeners, inFlight }` per key.
- `subscribe` adds a listener; the first listener starts loading and wiring refresh triggers
  (`window` `focus`, `visibilitychange`, `subscribeGitChanged` from
  `src/platform/tauri/fs.ts`); the last unsubscribe tears them down.
- `publish` skips notifying when the new value is structurally equal (`branchesEqual`), so
  `useSyncExternalStore` sees a stable reference.
- Loading is skipped while `document.hidden` unless forced; `inFlight` prevents overlap.
- `subscribe` and `getSnapshot` are wrapped in `useCallback` keyed on the inputs; a disabled
  hook returns a constant (`PENDING`) and a no-op unsubscribe.
- A `seedXxx` export lets another webview prime the cache (`seedProjectBranches`).

`getSnapshot` must return the same object until data changes. For values parsed from
localStorage, cache the raw string and parsed value (see `prefStore` in
`src/features/mail/model/mailPrefs.ts`).

## Pattern 2: one-off lookup per input (`useEffect` + disposed flag)

Reference: `src/features/telegram/hooks/useTelegramLinkable.ts`.

```ts
useEffect(() => {
  if (!project) return;
  let disposed = false;
  void telegramStatus(project).then(
    (status) => { if (!disposed) setLinkable({ project, value: /* ... */ }); },
    () => { if (!disposed) setLinkable({ project, value: false }); },
  );
  return () => { disposed = true; };
}, [project]);
return !!project && linkable?.project === project && linkable.value;
```

Rules visible here and repeated across hooks:
- Store the input key with the result and compare it on read, so a stale answer for a
  previous input is never returned.
- Guard every async `setState` with a `disposed`/`active` flag set in the cleanup.
- Handle rejection explicitly with a safe fallback; never leave a floating promise (use
  `void` for intentionally unawaited calls).
- Fetch only while needed (the hook runs only when `project` is non-null, so a menu pays for
  the lookup only while open).

## Pattern 3: per-session subscriptions

Components rendering one session use `useSessionSnapshot(fallback, visible)` from
`src/features/sessions/model/sessionStateStore.ts`; hidden panes keep their last value and do
not subscribe. Do not read the whole `sessions` array in a per-session component.

## Stabilising derived values

`useStableValue(value, equal = plainEqual)` in `src/shared/hooks/useStableValue.ts` returns
the previous reference while the value is structurally equal. Use it for arrays/objects
derived from streaming session state before passing them to memoised children.

## Events inside hooks

- Native Tauri events: subscribe through the platform wrappers or `listen` (see
  platform-bridge.md) and return the `UnlistenFn` from cleanup; `listen` is async, so guard
  against unmount before the promise resolves.
- In-window change notifications use `window` `CustomEvent`s named `monocode:<thing>-change`
  (state-management.md).

## Testing hooks

Hooks are tested in happy-dom with a tiny probe component, no testing-library:

```ts
// @vitest-environment happy-dom
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
function Probe({ project }: { project: string | null }) {
  latest = useTelegramLinkable(project);
  return null;
}
await act(async () => root.render(createElement(Probe, { project })));
```

See `src/features/telegram/hooks/useTelegramLinkable.test.ts` and quality-guidelines.md for
the `IS_REACT_ACT_ENVIRONMENT` setup.
