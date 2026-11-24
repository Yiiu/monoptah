# Type Safety

TypeScript ~5.8, `strict`, no runtime validation library (no zod/io-ts in `package.json`).
External data is narrowed by hand from `unknown`.

## Compiler settings that matter (`tsconfig.json`)

- `strict`, `noUnusedLocals`, `noUnusedParameters`, `noFallthroughCasesInSwitch`:
  `npx tsc --noEmit` fails on an unused import, local or parameter; delete unused code.
- `moduleResolution: "bundler"`, `allowImportingTsExtensions`, `isolatedModules`,
  `jsx: "react-jsx"`. Type-only imports use `import type { ... }` (or inline `type X`),
  e.g. the session imports at the top of `src/features/sessions/data/sessionStore.ts`.
- **`exclude: ["src/**/*.test.ts"]`: tests are not type-checked by `tsc`.** Vitest strips
  types without checking. A test with a type error still runs green, so keep test code
  honest by hand (no `as never` shortcuts to silence wrong fixtures).
- `src/vite-env.d.ts` references `vite/client`, which types `import.meta.env`,
  `import.meta.glob` and `?raw` imports (`import app from "./App.tsx?raw"` in
  `src/app/mailWiring.test.ts`).

## Where types live

- Domain types sit in the feature's `model/` next to the logic: `Session`, `Block`,
  `HarnessId`, `RuntimeMode` in `src/features/sessions/model/session.ts`.
- Harness wire events: discriminated union `HarnessEvent` (`{ type: "turn.started"; ... }`) in
  `src/integrations/harness/core/types.ts`; adapter contract `HarnessAdapter` in
  `core/registry.ts`.
- Command payload/return types are declared next to their `invoke` wrapper
  (`FsEntry`, `ProjectLocation` in `src/platform/tauri/fs.ts`; `SessionSummary` in
  `src/features/sessions/data/sessionStore.ts`). They must mirror the Rust `serde` structs
  (camelCase fields); nothing checks this automatically.
- Component props: local `type Props` (component-guidelines.md).

## Closed sets: union + const array

Closed string sets are a union plus a runtime list used for validation:

```ts
export type HarnessId = "claude" | "codex" | /* ... */ "devin";
export const HARNESSES: HarnessId[] = ["claude", "codex", /* ... */ "devin"];
```

(`src/features/sessions/model/session.ts`; same for `RuntimeMode` / `RUNTIME_MODES`.) For a
subset, derive the type from the array and pin it with `satisfies`:

```ts
export const PROVIDER_ACCOUNT_PROVIDERS = ["claude", "codex", "devin"] as const satisfies readonly HarnessId[];
export type ProviderAccountProvider = (typeof PROVIDER_ACCOUNT_PROVIDERS)[number];
```

(`src/features/providers/model/providerAccounts.ts`). Key maps use `as const`
(`MAIL_PREF_KEYS`, `LAYER`). Adding a member to a union means updating its array too, or
restored data with the new value is dropped as invalid.

## Validating external data

Data from localStorage, disk, Rust JSON blobs, provider CLIs and remote hosts is treated as
`unknown` and narrowed field by field:

- Object guard: `asRecord(value): Record<string, unknown> | null`
  (`src/integrations/harness/providers/claude/claudeProtocol.ts`) or
  `isRecord(value): value is Record<string, unknown>`
  (`src/features/ticket-contributors/model/prefs.ts`). Several modules keep a local copy;
  reuse the nearest one in the same area rather than adding another.
- Read each field with `typeof` checks and defaults; filter arrays with type predicates
  (`strings()` in `ticket-contributors/model/prefs.ts`).
- Check enum-like strings against the runtime list before casting:
  `HARNESSES.includes(record.harness as HarnessId) ? (record.harness as HarnessId) : undefined`
  (`src/features/sessions/data/sessionStore.ts`). The cast is allowed only after the check.
- Persisted sessions pass through sanitisers before use or save
  (`sanitizeSessionForPersist`, `sanitizeLinkedWorkItem`, `sanitizeQueuedMessages`).
- Tolerate old shapes on read and write the new shape (`parseExcludedMembers` accepts
  legacy bare ids).
- Provider protocol parsers (`providers/<id>/<id>Protocol.ts`) must never throw on
  unexpected input: return `null`/skip, as `decodePtyChunk` does in
  `src/platform/tauri/pty.ts`.
- Extracting JSON from model text: `extractJsonObject` in `src/shared/lib/jsonText.ts`, then
  guard the parsed value.

## Forbidden

- `any` (there is effectively none in non-test code) and `@ts-ignore` /
  `@ts-expect-error` in `src/` (zero today; the only one is in `vite.config.ts`).
- Casting an `invoke` result or `JSON.parse` output to a domain type without checks when it
  comes from storage written by an older version or another machine.
- Non-null assertions (`!`) on data from outside; they appear only where the invariant is local
  and obvious (e.g. `monoPersistedBlock(block)!` after filtering).
