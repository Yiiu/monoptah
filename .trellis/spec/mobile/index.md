# Mobile (`mobile/`) Guidelines

The phone app: an Expo / React Native project (Expo SDK 57, React Native 0.86, React 19.2,
Expo Router) that pairs with a desktop over direct, mutually authenticated QUIC and shows that
desktop's projects, monos and sessions. It is a **separate npm project** with its own
`mobile/package.json` and `mobile/package-lock.json`; the root project is not involved except
for running its tests.

## Authoritative docs (read, do not duplicate)

- [`mobile/AGENTS.md`](../../../mobile/AGENTS.md) (also loaded by `mobile/CLAUDE.md`):
  Expo changes every SDK, so check the `expo` major in `package.json` and read the matching
  versioned docs before using any Expo/RN API; add packages with `npx expo install`; routes
  live only in `src/app/`; never hand-edit generated `ios/`/`android/` (configure via
  `app.json` and config plugins); a native module needs a development build, not Expo Go.
- [`mobile/README.md`](../../../mobile/README.md): run locally (`npm run ios` builds the Rust
  link first), pairing, EAS profiles, what works on the phone.

## What this layer owns

- Screens and navigation: `mobile/src/app/**` (Expo Router).
- Feature code: `mobile/src/features/<area>/` (connection, workspace, sessions, changes,
  files, models, settings, notifications, visuals, background, mascots).
- Shared UI primitives and theme: `mobile/src/ui/`.
- The Expo local native module `mobile/modules/monocode-link` (Swift/Kotlin) that wraps the
  Rust crate `crates/mobile-link` (`monocode-mobile-link`), built by
  `mobile/scripts/build-native-link.mjs`.
- Config plugin `mobile/plugins/with-scene-lifecycle.js`, `app.json`, `eas.json`.

It does **not** own the desktop side of the protocol: the native app answers in
`crates/monocode-core/src/mobile/` (`host.rs` `HOST_METHODS`/`APP_METHODS`, `policy.rs`) and
`crates/monocode-gpui/src/shell/mobile_host.rs`; the Tauri app in `src-tauri/src/mobile_access.rs`
and `src/features/mobile/`. `web/protocol/src/methods.ts` reads `mobile/src/features/**` for
result shapes.

## Pre-Development Checklist

1. Read `mobile/AGENTS.md` and check the Expo SDK major before any Expo/RN API change.
2. Read [structure.md](./structure.md) for where code goes and platform-file conventions.
3. Read [state-and-networking.md](./state-and-networking.md) before touching
   `ConnectionProvider`, `WorkspaceProvider`, `nativeLink.ts`, `watchUpdates.ts`, storage keys,
   or any RPC method name or payload.
4. Read [testing.md](./testing.md) before adding tests.
5. If the change needs a desktop method, find it in `crates/monocode-core/src/mobile/host.rs`
   first; a new method needs a desktop change and a permission class in `policy::METHODS`.
6. Keep UI copy and accessibility labels in English.

## Quality Check

CI's "Mobile checks" job (`.github/workflows/ci.yml`, currently `workflow_dispatch` only, so
run these yourself):

```bash
npm ci && (cd mobile && npm ci)            # both trees: tests import root and mobile deps
(cd mobile && npm run lint)                 # expo lint (eslint-config-expo flat config)
(cd mobile && npx tsc --noEmit)
npx vitest run --config mobile/vitest.config.ts   # from the repo root; vitest is a root dependency
```

`npx expo-doctor` diagnoses dependency drift. Native code changes (Swift/Kotlin, Rust link,
`app.json`, plugins) need a rebuilt development app (`npm run ios` / `npm run android`, or
`npx expo prebuild --clean` first); EAS Update alone cannot ship them.

## Files

| Path | Role |
|---|---|
| `src/app/_layout.tsx` | Root providers (`ConnectionProvider`, `WorkspaceGate`, `WorkspaceProvider`, `WorkspaceLayout`), stack options per theme |
| `src/app/**` | Thin route files: read params, resolve with helpers, render a feature screen |
| `src/features/connection/` | `nativeLink.ts` (native bridge, `request`, watches), `ConnectionProvider.tsx` (pairing, catalog poll), `invitation.ts`, `selection.ts`, `use*` data hooks |
| `src/features/workspace/` | `WorkspaceProvider.tsx` (conversations, watches, mutations, turn cache), `watchUpdates.ts` (frame patches), `routing.ts`, layout, drawer, sidebar |
| `src/features/sessions/` | Transcript, composer, session screen, cards, `activityStore.ts`, `NativeTranscript.tsx` |
| `src/features/models/` | Model and account pickers, `useModelCatalog`, `sessionChoice.ts` |
| `src/features/settings/` | Small persisted settings stores (`toolCalls.ts`, `appearance.ts`, ...) |
| `src/features/changes/`, `files/` | Git changes, diffs, commit sheet; file tree, code and document viewers |
| `src/ui/` | Shared primitives (`ModalSheet`, `ChoiceMenu`, `LiquidView`, `theme.ts`, ...) |
| `modules/monocode-link/` | Expo module `MonocodeLink` (iOS native transcript, typing posture, scroll edge effects) |
| `scripts/build-native-link.mjs` | `cargo build -p monocode-mobile-link` for iOS/Android targets |
| `scripts/eas-build-pre-install.mjs` | EAS hook: installs Rust and builds the link before pods/Gradle |
| `tests/*.test.ts(x)`, `../tests/mobile/*.test.ts` | Vitest suites (see testing.md) |
| `vitest.config.ts`, `tsconfig.json`, `eslint.config.js` | Test, compiler (`@/*` → `src/*`) and lint config |

## Topic Files

| Guide | Covers |
|---|---|
| [structure.md](./structure.md) | Routes vs features vs ui, platform files, native module, identity and branding notes |
| [state-and-networking.md](./state-and-networking.md) | Native link calls, pairing, polling, watches, idempotent mutations, storage keys, old-desktop compatibility |
| [testing.md](./testing.md) | Vitest config, mocking React Native and Expo, provider tests, cross-tree round-trip tests |
