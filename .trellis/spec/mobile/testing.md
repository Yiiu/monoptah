# Mobile Testing

## How tests run

- Command, from the repository root:
  `npx vitest run --config mobile/vitest.config.ts` (the CI "Mobile checks" job in
  `.github/workflows/ci.yml` runs exactly this after `npm ci` in both the root and `mobile/`).
  Vitest and `happy-dom` are root dependencies; `mobile/package.json` has neither, and the
  config aliases `react` and `react-dom` to `mobile/node_modules`, so both installs are needed.
- `mobile/vitest.config.ts`: `environment: "happy-dom"`, `define: { __DEV__: false }`
  (Metro's global), `@` → `src`, automatic JSX, one worker (`maxWorkers: 1`, `minWorkers: 1`).
- Included: `mobile/tests/**/*.test.ts(x)` and `tests/mobile/**/*.test.ts` at the repo root.
  The root folder holds round trips that need both trees: `tests/mobile/watchFrames.test.ts`
  feeds the desktop's `watchUpdate` (`src/features/mobile/model/mobileWatch`) into the phone's
  `applyFrames`. Changes to the watch frame format need a case there.
- Lint and types are separate: `cd mobile && npm run lint` and `npx tsc --noEmit`.
- Tests are not colocated: they live in `mobile/tests/`, named after the unit
  (`Composer.test.tsx`, `watchUpdates.test.ts`, `routing.test.ts`).

## Pure logic first

Helpers without React are tested directly, e.g. `tests/connection.test.ts`
(`normalizeSelection`, `selectedWorkspace`, `parseInvitation`), `tests/watchUpdates.test.ts`,
`tests/markdown.test.ts`, `tests/routing.test.ts`, `tests/modelChoice.test.ts`. Keep
protocol parsing and patching in such modules so it can be tested this way.

## Rendering components

There is no React Native renderer. Tests render with `react-dom/client` into happy-dom and
mock React Native and Expo modules with DOM stand-ins:

```tsx
vi.mock("react-native", () => ({
  AppState: { currentState: "active", addEventListener: () => ({ remove() {} }) },
}));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean })
  .IS_REACT_ACT_ENVIRONMENT = true;
```

- Mock only what the unit imports; component tests map `View`/`Text` to `div`/`span`
  (`tests/Composer.test.tsx`). Shared state for mocks goes in `vi.hoisted(...)`.
- Drive updates with `act(() => root.render(...))` and `act(async () => ...)`.
- These tests prove behavior and component identity, not native layout or scroll offsets
  (`mobile/README.md`); native transcript and hinge behavior still need a simulator or device.

## Providers and the desktop link

`tests/WorkspaceProvider.test.tsx` shows how to test code that talks to the desktop without a
native module:

- Mock `@/features/connection/ConnectionProvider` so `useConnection()` returns a fixed
  `pairing`, `catalog`, `selection` and a `command: vi.fn()`.
- Mock `@/features/connection/nativeLink` (`openWatch`, `nextUpdates`, `closeWatch`,
  `makeCommandId`, and `hasNativeTranscript` as a getter so a test can flip it).
- Mock `expo-sqlite/kv-store` with an in-memory `Map` exposing `getItem`, `setItem`,
  `removeItem`.
- Use a `deferred()` promise for `nextUpdates` so the test decides when each watch frame
  arrives, then assert on rendered output.

Never call the real `MonocodeLink` module or a real desktop in tests.

## Expectations for a change

- New or changed RPC usage: a provider or hook test asserting the method name, params and
  command ID reuse on retry.
- New optional desktop field: one test with the field and one without (older desktop).
- Android-specific UI: an `Android*.test.tsx` case next to the shared one.
- UI copy and accessibility labels are English; tests assert the English text.
