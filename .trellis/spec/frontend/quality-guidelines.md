# Quality Guidelines

## Test setup (what actually runs)

- Unit tests: vitest, `vitest.config.ts` has `environment: "node"` and
  `include: ["src/**/*.test.ts"]`. **Only `.test.ts` files run**; a `.test.tsx` file is
  silently ignored (there are none). Tests therefore build elements with
  `createElement(...)`, not JSX.
- DOM tests opt in per file with the first line `// @vitest-environment happy-dom` (about 300
  files). No `@testing-library/*` is installed.
- Tests are excluded from `tsc` (`tsconfig.json`), so type errors in tests go unnoticed.
- Browser tests: Playwright, `playwright.config.ts`, `testDir: "./tests/browser"`, Chromium
  and WebKit, one worker, own Vite server on port 1435. Each spec has an HTML/TSX harness page
  (`tests/browser/transcript.html`, `session-pane.tsx`, ...). Run `npm run test:browser` for
  transcript layout, scrolling, tab reorder and title bar geometry, which happy-dom cannot
  measure (`CONTRIBUTING.md`).

## Test styles in use

1. **Pure logic** (most tests): import from `model/` and assert. Name files after the module,
   split big suites by topic (`sessionStoreRestore.test.ts`, `sessionStoreConcurrency.test.ts`).
2. **Static markup**: `renderToStaticMarkup(createElement(ModalPanel, {...}))` and
   `expect(markup).toContain('role="dialog"')` (`src/shared/ui/Modal.test.ts`). Use for
   accessibility attributes and copy.
3. **Mounted (happy-dom)**, from `src/shared/hooks/useAnimatedReorderConcurrency.test.ts`:

```ts
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
```

   Drive events with `act(() => target.dispatchEvent(new PointerEvent(...)))`; use
   `vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] })` for timers.
4. **Source-scan guards** read source text with `?raw`, `import.meta.glob` or `readFileSync`
   and fail on forbidden patterns: `src/shared/ui/icons.test.ts` (hugeicons only through the
   catalog), `src/shared/ui/focusVisibleHas.test.ts` (no `group-has-[:focus-visible]`),
   `src/features/mail/model/noTiptapPro.test.ts`, and the `App.tsx` wiring tests
   `src/app/mailWiring.test.ts`, `contributorsWiring.test.ts`,
   `contributorsEntryWiring.test.ts` (App.tsx is too large to mount). When one fails, fix the
   code, not the guard.

Tauri is always mocked in unit tests; see platform-bridge.md ("Mocking in tests").

## Commands

```bash
npx vitest run <changed test files>
npx tsc --noEmit
npm run check:web > web.log 2>&1; echo $?
```

Never read a check through a pipe (`npm run check | tail` always exits 0; repo `CLAUDE.md`,
"Verifying"). Never weaken an assertion to make a test pass.

## Fork branding (Monoptah)

This is a fork of `hardbeat920/monocode`. All user-visible copy (dialog titles, buttons,
tooltips, window title, "What's new in ...", errors) says **Monoptah**; links go to
`hhoangg/monoptah`, the Website item to `https://zptah.app`. Run
`./scripts/check-fork-branding.sh` (Git Bash on Windows) after editing copy and after every
upstream merge. It scans string literals in `src/`, ignores `.test.` files and comments, and
has an `ALLOW` list with reasons. Add to `ALLOW` only with a reason; never loosen `PATTERN`.
A test that asserts "MonoCode" copy must be updated, not kept.

Identifiers that look like branding but must not change (repo `CLAUDE.md`, "Never rename
these"):

- `monocode.*` localStorage keys and `monocode:*` window event names.
- `INTERRUPT_MESSAGE = "Turn interrupted when MonoCode quit."` in
  `src/features/sessions/model/inFlight.ts`: compared with `===` against text already stored
  in saved sessions (`inFlight.ts`, `transcriptActivity.ts`). The file is on the branding
  `ALLOW` list for this reason.
- Code identifiers such as `MonoCodeMark`, `performance.mark("monocode:bootstrap")`.

UI labels and accessibility text are English; user-authored text is kept as written.

## Forbidden patterns

| Pattern | Instead | Enforced by |
|---|---|---|
| Importing `@hugeicons/*` outside `src/shared/ui/icons.tsx` | Add a wrapper to the catalog | `icons.test.ts` |
| `group-has-[:focus-visible]` / `peer-has-[:focus-visible]` | Name the element in the selector | `focusVisibleHas.test.ts` |
| Raw `z-50`-style layers for floating UI | `LAYER` from `src/shared/lib/layers.ts` | review |
| Custom overlay/dialog markup | `Modal` / `Popover` | review |
| Native `<select>` in themed UI | Settings `Select`, `SearchableSelect` | review |
| Unguarded `localStorage` access | `try/catch` helpers (state-management.md) | review |
| Renaming a `monocode.*` key or `INTERRUPT_MESSAGE` | Keep; add a new versioned key | `CLAUDE.md` |
| New provider harness | Fix existing ones | `CONTRIBUTING.md` |
| Importing `src/` from `web/` or the reverse | Copy with provenance per the web rules | `scripts/check-web-isolation.mjs` |
| Realistic fake tokens in tests | `concat!`-style building or obvious fakes | GitHub push protection |
| `npm run tauri dev` | `npm run tauri:dev` (isolated identifier and data) | `CLAUDE.md` |

## Review checklist

- New logic lives in `model/` with a `.test.ts`; DOM tests declare happy-dom.
- Each `invoke` wrapper is typed and its test asserts the exact command name and arg keys.
- Async effects guard against unmount; listeners are removed in cleanup.
- Preferences: existing key reused or new `monocode.` key with try/catch and validation.
- User-visible text says Monoptah; `./scripts/check-fork-branding.sh` passes.
- If code imported by `host/` changed: `npm run test:host` passes.
- `npm run check:web` exit status is 0.
