# Mobile Structure

## Three layers under `mobile/src`

| Layer | Path | Rule |
|---|---|---|
| Routes | `src/app/**` | Every file is a screen; `_layout.tsx` files define navigators (`mobile/AGENTS.md`). Keep route files thin. |
| Features | `src/features/<area>/` | Screens, hooks, providers and pure helpers for one area. |
| UI | `src/ui/` | App-wide primitives and theme, with no knowledge of the desktop protocol. |

Imports use the `@/` alias (`@/*` → `./src/*` in `mobile/tsconfig.json`, mirrored by
`resolve.alias` in `mobile/vitest.config.ts`), e.g.
`import { useWorkspace } from "@/features/workspace/WorkspaceProvider"`.

### Route files are adapters

`src/app/project/[projectId]/session/[sessionId].tsx` shows the pattern: read params with
`useLocalSearchParams`, resolve them through a feature helper, redirect when missing, render a
feature component.

```tsx
const project = projectForRoute(useWorkspace().projects, projectId);
if (!project) return <Redirect href="/" />;
return <SessionFrame ...><ProjectScreen project={project} sessionId={sessionId} /></SessionFrame>;
```

`projectForRoute` (`src/features/workspace/routing.ts`) exists because desktop project IDs are
encoded paths and Expo Router decodes params; always resolve route project IDs through it and
use the catalog's canonical `id` for RPCs.

### Root composition (`src/app/_layout.tsx`)

`ConnectionProvider` → `WorkspaceGate` (pairing gate) → `WorkspaceProvider` →
`WorkspaceLayout` (one stack; projects/recent sessions sidebar at 740 points of usable width or
more, compact navigation below, per `mobile/README.md`). Stack screen options come from
helpers (`codeScreenOptions`, `modalStackOptions`, `modalSheetOptions`, `HEADER_OWN_BLUR`) and
the navigation theme is derived from `paletteFor`/`useTheme` in `src/ui/theme.ts`, so new
screens should reuse those option sets instead of hard-coding colors.

## Feature areas

- `connection/`: the native bridge and pairing (`nativeLink.ts`, `ConnectionProvider.tsx`,
  `invitation.ts`, `selection.ts`) and per-project data hooks (`useProjectChanges`,
  `useProjectGit`, `useProjectWorktrees`, `useFileTree`, `useFileSearch`).
- `workspace/`: `WorkspaceProvider.tsx` (conversations and mutations), `watchUpdates.ts`,
  layout/sidebar/drawer, mono home and profile helpers, `lastPlace.ts`, `monoSeen.ts`.
- `sessions/`: transcript (`Transcript.tsx`, `TranscriptTurn.tsx`, `NativeTranscript.tsx`),
  `Composer.tsx`, `SessionScreen.tsx`, cards, `activityStore.ts`, markdown parsing.
- `models/`, `settings/`, `changes/`, `files/`, `notifications/`, `visuals/`,
  `background/`, `mascots/`: self-contained areas named after what they show.

Small cross-screen state that is not desktop data is a module-level store read with
`useSyncExternalStore` and persisted in `expo-sqlite/kv-store` (`src/features/settings/toolCalls.ts`,
`src/features/sessions/activityStore.ts`, `src/features/models/sessionChoice.ts`). Follow that
pattern instead of adding a new context for a single setting.

## Platform files

Platform variants use React Native's file suffix: `WorkspaceMenu.android.tsx` beside
`WorkspaceMenu.tsx`, `SegmentedSwitch.android.tsx` beside `SegmentedSwitch.tsx`. Smaller
differences use `Platform.OS` checks inline (about 20 files). Both platforms must keep working:
Android has tests of its own (`tests/AndroidHeader.test.tsx`,
`tests/AndroidSegmentedSwitch.test.tsx`, `tests/AndroidWorkspaceLine.test.tsx`).

## Native module and Rust link

- `modules/monocode-link` is an Expo local module named `MonocodeLink`
  (`expo-module.config.json`; `ios/MonocodeLinkModule.swift`,
  `android/.../MonocodeLinkModule.kt`). JS loads it with
  `requireOptionalNativeModule<NativeLink>("MonocodeLink")` in `nativeLink.ts`, so a build
  without it still renders and `nativeCall` throws "Rebuild the mobile app to enable desktop
  connections."
- Native features are gated by version fields the module exports, not by app version:
  `hasNativeTranscript = Platform.OS === "ios" && module?.nativeTranscriptVersion === 1`,
  likewise `hasTypingPosture`. Older development builds and Android fall back to the
  JS transcript. Bump the version and keep the fallback when changing a native contract.
- The transport is Rust (`crates/mobile-link`, crate `monocode-mobile-link`), compiled by
  `scripts/build-native-link.mjs` (`cargo build --locked --release -p monocode-mobile-link`
  per target; iOS framework `MonocodeLinkCore.framework`, deployment target 16.4).
  EAS builds run it from `scripts/eas-build-pre-install.mjs`.
- Generated `ios/` and `android/` folders are ignored by Git; configure native behavior in
  `app.json` and `plugins/with-scene-lifecycle.js`.

## Identity and branding

`mobile/package.json` is `monocode-mobile`; `app.json` still carries upstream identity (`name`
"MonoCode", `owner` "hardbeat920", `scheme` "monocode", bundle id
`com.hardbeat920.monocode-mobile`). `scripts/check-fork-branding.sh` does not scan `mobile/`.
Do not change the bundle id, scheme, or EAS `owner`/`slug` as a drive-by edit: they decide
which app installs update and which EAS project builds. Rebranding the mobile app is a
deliberate, separate task.
