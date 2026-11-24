# Directory Structure

The frontend is feature-sliced, with relative imports only (no `@/` alias in `tsconfig.json`)
and almost no barrel files. Tests sit next to the file they test.

## Top level of `src/`

```
src/
  main.tsx               boot: appearance, sounds, homeDir, lazy-imports ./app/App
  app/
    App.tsx              composition root (~16k lines, ~80 useState); default export
    shell/               chrome: TitleBar, ProjectRail, Sidebar, WindowControls, UsageFooter
    model/               app-level logic: appLifecycle, quitGate, updater, windowTransfer
    hooks/               app-level hooks: useWorkspaceNavigation, useMonoWorkflows
    ui/                  ReleaseNotesSurface
  features/<name>/       product behavior (26 features)
  integrations/harness/  provider-independent core/ + providers/<id>/ adapters
  platform/tauri/        fs.ts, pty.ts, clipboard.ts, platform.ts, trackpadZoom.ts
  shared/ui|hooks|lib/   reusable, feature-free building blocks
  styles/index.css       the single stylesheet (Tailwind v4 + tokens)
```

Extra Vite entry pages are declared in `vite.config.ts` (`build.rollupOptions.input`):
`index.html`, `quick-composer.html` (boots `src/features/quick-composer/main.tsx` without the
workspace) and `mono-chat.html`. A new separate window needs its own entry there.

## Inside a feature

`src/features/sessions/` is the reference layout:

| Folder | Holds | Example |
|---|---|---|
| `model/` | Types, pure logic, constants, and often thin `invoke` wrappers | `model/session.ts` (`Session`, `HarnessId`, `HARNESSES`), `model/inFlight.ts` |
| `data/` | Persistence through Rust | `data/sessionStore.ts` (`session_upsert`, `session_list_by_project`) |
| `hooks/` | React hooks for the feature | `hooks/useTranscriptLayout.ts`, `hooks/useUnseenReplyCounts.ts` |
| `ui/` | Components (`.tsx`) and UI-only helpers | `ui/HarnessIcon.tsx`, `ui/DeleteSessionDialog.tsx` |

Not every feature has every folder (`agent-app/` has only `model/`; `notes/` and
`artifacts/` keep files at the feature root). `model/` is the default home for logic: 40
model files call `invoke` directly, versus one in `data/`. Prefer putting React-free logic in
`model/` so it can be tested in the node environment and reused by `host/`.

## Harness adapters

Each provider folder in `src/integrations/harness/providers/<id>/` follows the pattern
described in `CONTRIBUTING.md`: `<id>Adapter.ts` implements `HarnessAdapter` from
`core/registry.ts`, and `<id>Protocol.ts` holds pure parsing with `<id>Protocol.test.ts`
beside it (e.g. `providers/claude/claudeProtocol.ts`). The package entry is
`src/integrations/harness/index.ts`, one of the few barrels. Upstream is not accepting new
providers; do not add one.

## Where new code goes

- New product area: `src/features/<kebab-name>/` with `model/` + `ui/` (+ `hooks/` if needed).
- Logic used by one feature: that feature's `model/`. Used by several unrelated features and
  free of feature types: `src/shared/lib/`.
- A reusable visual primitive: `src/shared/ui/`. A primitive that only Settings uses stays in
  `src/features/settings/ui/primitives.tsx` (`Group`, `Row`, `Toggle`, `Select`, `Segmented`).
- A new Tauri command wrapper used across features: `src/platform/tauri/fs.ts` (or a sibling);
  feature-specific commands: the feature's `model/` file.
- A big optional view: load it through `lazySurface` in `App.tsx` (see
  component-guidelines.md).

File naming: components `PascalCase.tsx`, everything else `camelCase.ts`, tests
`<name>.test.ts` (some split by topic: `sessionStoreRestore.test.ts`,
`useTicketBoard.reclassify.test.ts`).

## Import directions (as they really are)

| From | May import | Notes |
|---|---|---|
| `shared/` | `shared/`, React, libs | Exceptions exist: `shared/ui/Modal.tsx` imports `app/shell/GlassBackdrop`; `shared/ui/ImageLightbox.tsx`, `PrivateEmail.tsx`, `hooks/useColorScheme.ts` reach into `features/`. Do not add more. |
| `platform/tauri/` | `shared/lib`, Tauri APIs, feature **types** | `fs.ts` imports `type InterjectionMeta` from sessions; keep it type-only. |
| `features/<a>/` | other features' modules directly, `shared/`, `platform/`, `integrations/` | Deep relative paths, e.g. `../../sessions/model/session`. 32 feature files import `app/shell/TitleBar`, `WindowControls`, `GlassBackdrop`. |
| `integrations/harness/` | `features/*/model` types and helpers, `platform/` | Must stay usable headless: process I/O goes through `ChildBackend` (`core/child.ts`). |
| `app/` | everything | Only `App.tsx` and `main.tsx` wire features together. |

Hard rules:
- Never import from `web/` into `src/` or the reverse; `scripts/check-web-isolation.mjs`
  (CI job `web-guard.yml`) fails on any `web/` reference into `src/`.
- Code imported by `host/` (see `index.md`) is bundled by esbuild in `host/build.mjs`
  (`platform: "node"`) into `build/host/monocode-host.mjs`. It must not touch `window`,
  `document` or `localStorage` at module top level, and harness process I/O must go through
  the `invoke`/`listen` wrappers in `core/child.ts` so `configureChildBackend` can swap in
  the Host's backend.
