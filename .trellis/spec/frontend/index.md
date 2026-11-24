# Frontend (Tauri web app) Guidelines

Specs for the React + TypeScript + Vite frontend of the Tauri desktop app: everything
under `src/`, its vitest unit tests (`src/**/*.test.ts`) and its Playwright browser tests
(`tests/browser/`). Read this file first, then the topic files you need.

## What this layer owns

| Area | Path | Role |
|---|---|---|
| Composition root | `src/app/App.tsx` (~16k lines), `src/main.tsx` | Boot, workspace state, view routing, wiring every feature |
| App chrome | `src/app/shell/`, `src/app/model/`, `src/app/hooks/` | Title bar, rails, sidebar, updater, quit gate, lifecycle |
| Product features | `src/features/<feature>/{model,ui,hooks,data}` | Sessions, projects, files, settings, inbox, mail, ... |
| Provider harnesses | `src/integrations/harness/{core,providers/<id>}` | Talk to Claude, Codex, Cursor, ... CLIs; protocol parsing |
| Tauri adapters | `src/platform/tauri/` | Typed wrappers for `invoke`/`listen`, PTY, clipboard, OS flags |
| Reusable pieces | `src/shared/{ui,hooks,lib}` | Modal, Popover, icons, z-layers, small utilities |
| Styles | `src/styles/index.css` | Tailwind v4 entry, `@theme` tokens, global classes |

Status: releases ship the native GPUI app only. The Tauri workflows are paused
(`workflow_dispatch` only), but this code is still maintained and merged from upstream
`hardbeat920/monocode`, so keep it building and tested. See the repo `CLAUDE.md`.

Shared consumers outside `src/`: the remote Host (`host/*.ts`) imports
`src/integrations/harness/**`, `src/features/{connections,sessions,search,source-control}/model/**`
and `src/platform/tauri/fs`. A change there also affects `npm run test:host`.

## Pre-Development Checklist

1. Read [directory-structure.md](./directory-structure.md) to place new files.
2. Touching UI: [component-guidelines.md](./component-guidelines.md); reuse `src/shared/ui/*`.
3. Adding state or a preference: [state-management.md](./state-management.md). Never rename
   a `monocode.*` storage key.
4. Calling Rust or listening to a native event: [platform-bridge.md](./platform-bridge.md).
5. Parsing anything from disk, localStorage, a CLI, or Rust: [type-safety.md](./type-safety.md).
6. Any user-visible text: it says "Monoptah", never "MonoCode" (see
   [quality-guidelines.md](./quality-guidelines.md)). Keep `INTERRUPT_MESSAGE` in
   `src/features/sessions/model/inFlight.ts` byte-identical.
7. Changing `src/integrations/harness/` or a model file imported by `host/`: also run the Host
   checks listed below.
8. Cross-layer change (new Tauri command): read `../guides/cross-layer-thinking-guide.md` and the
   backend spec in `../backend/`.

## Quality Check

Run each command and read its own exit status. Never pipe it (`| tail` hides failures):

```bash
npx vitest run src/features/sessions/model/inFlight.test.ts   # the files you touched
npx tsc --noEmit                                              # app types (tests are excluded!)
npm run check:web > web.log 2>&1; echo $?                     # vitest run && tsc --noEmit
```

Also, when relevant:

- `./scripts/check-fork-branding.sh` after editing user-visible copy or merging upstream.
- `npm run test:host` (builds and type-checks `host/` first) when touching code `host/` imports.
- `npm run test:browser` for transcript layout, scrolling, tab or title-bar geometry
  (needs `npx playwright install chromium webkit` once).
- `npm run tauri:dev` to see a change live with isolated dev data. Never plain
  `npm run tauri dev` (it shares the installed app's data).

`npm run check` is the full suite (web + Rust); use it before a PR, not per edit.

## Guidelines Index

| File | Covers |
|---|---|
| [directory-structure.md](./directory-structure.md) | Layout, where new code goes, real import directions |
| [component-guidelines.md](./component-guidelines.md) | Components, shared primitives, Tailwind tokens, accessibility |
| [hook-guidelines.md](./hook-guidelines.md) | Custom hooks, module-level stores with `useSyncExternalStore` |
| [state-management.md](./state-management.md) | `SessionStateStore`, App state, localStorage prefs, window events |
| [platform-bridge.md](./platform-bridge.md) | `invoke`/`listen` wrappers, remote routing, mocking Tauri in tests |
| [type-safety.md](./type-safety.md) | TS config, unions + const arrays, parsing `unknown` |
| [quality-guidelines.md](./quality-guidelines.md) | Test styles, forbidden patterns, fork branding, review checklist |
