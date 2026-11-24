# Component Guidelines

React 19 function components, Tailwind v4 utility classes, no component library. Shared
primitives live in `src/shared/ui/`; reuse them before writing a new popup, dialog, or icon.

## Shape of a component

- Named exports, props typed with a local `type Props = { ... }` (116 `.tsx` files use exactly
  this). Default exports exist only for `src/app/App.tsx` and `src/features/mail/ui/MailEditor.tsx`.
- React 19 ref-as-prop: accept `ref?: Ref<HTMLDivElement>` in `Props` (see `Popover` in
  `src/shared/ui/Popover.tsx`). `forwardRef` remains only in older code such as the icon
  `wrap()` in `src/shared/ui/icons.tsx`.
- Wrapping a native element: extend its props, e.g. `SecondaryButton` in
  `src/shared/ui/SecondaryButton.tsx` uses `Omit<ComponentPropsWithRef<"button">, "className">`
  and defaults `type = "button"`. Buttons set `type="button"` explicitly (the overwhelming majority of the code does).
- Document non-obvious props with a `/** ... */` comment explaining *why*, as `Modal`'s
  `width`, `hideClose`, `fitViewport` do in `src/shared/ui/Modal.tsx`.
- Logic that can be pure goes into the feature's `model/` and is tested there; `ui/` keeps
  rendering and event wiring.

## Shared primitives to reuse

| Need | Use | File |
|---|---|---|
| Dialog | `Modal` (portal + scrim) / `ModalPanel` (inside a native popup) | `src/shared/ui/Modal.tsx` |
| Anchored menu, dropdown, picker | `Popover` with `anchor`, `onDismiss`, `layer` | `src/shared/ui/Popover.tsx` |
| Searchable dropdown | `SearchableSelect` | `src/shared/ui/SearchableSelect.tsx` |
| Settings rows and controls | `Group`, `Row`, `Toggle`, `Select`, `Segmented`, `Slider` | `src/features/settings/ui/primitives.tsx` |
| Icons | Named wrappers (`Check`, `ChevronDown`, `X`, ...) | `src/shared/ui/icons.tsx` |
| Bordered secondary action | `SecondaryButton` (`danger` variant) | `src/shared/ui/SecondaryButton.tsx` |
| z-index | `LAYER.popover / submenu / dialog / dialogPopover / toast / peek` | `src/shared/lib/layers.ts` |
| Lazy optional view | `lazySurface(load)` + `.preload()` | `src/shared/ui/lazySurface.tsx` |
| Pane header, tab label, shimmer, reveal | `PaneHeader`, `TabLabel`, `Shimmer`, `RevealHeight` | `src/shared/ui/` |
| Glass surface | `GlassBackdrop` | `src/app/shell/GlassBackdrop.tsx` |

Rules that come with them:

- **Icons only through `src/shared/ui/icons.tsx`.** `src/shared/ui/icons.test.ts` fails if any
  other file imports `@hugeicons/*`. Add a glyph by deep-importing
  `@hugeicons/core-free-icons/<Name>Icon` in the catalog and exporting `wrap(Icon, "Name")`.
  Default `strokeWidth` is 1.75.
- **No hard-coded z-index for floating UI.** Pick from `LAYER`; panels inside a surface stay
  below 50 (comment in `layers.ts`).
- **Dialogs**: `Modal` requires `title` (used for `aria-labelledby` even with
  `minimalHeader`, which renders it `sr-only`), handles Escape in the capture phase, and defers
  Escape to an open `[data-dialog-popover]`. It provides `InDialog` so a `Popover` opened inside
  lifts itself to `LAYER.dialogPopover`. Do not build a custom overlay.
- **No native `<select>`** for themed UI: the Settings `Select` doc comment explains that the
  OS-rendered option popup is unreadable in dark mode on Windows/Linux. Use `Select` or
  `SearchableSelect`.
- Big, optional surfaces (Search, Settings, Inbox, ...) are created with `lazySurface` in
  `App.tsx`, mapping a named export to `{ default: module.X }`:

```tsx
const SettingsView = lazySurface(async () => {
  const module = await import("../features/settings/ui/SettingsView");
  return { default: module.SettingsView };
});
```

## Styling

- Tailwind v4 via `@tailwindcss/vite`; the only stylesheet is `src/styles/index.css` (no CSS
  modules, no CSS-in-JS). Global class names (`modal-panel`, `modal-backdrop`, `settings-row`)
  are defined there.
- Use theme tokens from `@theme` in `index.css`, not raw colors: `bg-background-base`,
  `text-content` with opacity (`text-content/50`, `border-content/10`), `border-stroke`,
  `bg-selection*`, `accent`, `skill`, `mention`, diff tokens. Typical sizes are literal
  (`text-[12px]`, `size-7`).
- Dark mode is a custom variant: `dark` means `html:not(.theme-light)` (see
  `@custom-variant dark` in `index.css`); do not use `prefers-color-scheme` in components.
- WebKit performance guard: never write `group-has-[:focus-visible]` or
  `peer-has-[:focus-visible]`; `src/shared/ui/focusVisibleHas.test.ts` scans all sources for it.
  `group-focus-within` is redefined in `index.css` for the same reason.
- Diff UI must show a `+`/`-` glyph, color is never the only cue (comment on diff tokens in
  `index.css`).

## Accessibility conventions in use

- Icon-only buttons carry `aria-label` (Modal close button: `aria-label="Close"`).
- Toggles are `<button role="switch" aria-checked>` (`Toggle` in settings `primitives.tsx`),
  with `aria-describedby` for an explanation.
- Dialogs: `role="dialog" aria-modal="true" aria-labelledby aria-describedby` with ids from
  `useId()` (`ModalPanel`).
- Focus rings: `focus-visible:ring-2 focus-visible:ring-accent` or
  `focus-visible:outline-2 focus-visible:outline-accent`.
- Keyboard handlers that act on Enter/keys must ignore IME composition: use
  `isImeComposition` from `src/shared/lib/keyboard.ts`.
- Labels and accessibility text are English; user-authored text keeps its language.

## Performance habits visible in the code

- `App.tsx` is too large to mount in a test; its wiring is checked on source text
  (`src/app/mailWiring.test.ts`). When you add a view, add its close call next to the others
  so those tests stay green.
- Streaming sessions re-render often: subscribe per session through
  `useSessionSnapshot` (state-management.md) and stabilise derived values with
  `useStableValue` (`src/shared/hooks/useStableValue.ts`) instead of adding broad `memo`.
