# Agent CLIs settings page

Child 2 of `11-24-agent-settings-redesign`. Depends on child `11-24-agent-config-core` being
merged. Requirements: parent `prd.md`, `design.md` §7 and §9, `implement.md` ("Child 2"), and the
approved Ryugi artboard `.ryugi/artifacts/artboard/agent-clis-settings` (v2).

Native only. Strictly follow `.trellis/spec/native/component-guidelines.md`: reuse shared widgets;
new pieces go into the shared layer; colours only from `theme.rs`; refined, elegant, simple.

## Acceptance criteria
- [ ] Page matches artboard v2 structure and states in light and dark themes, using existing widget metrics.
- [ ] Permission icon/colour shared with the composer access menu.
- [ ] Every interaction in parent PRD "Layout and interaction" works; layout tests cover them.
- [ ] Search finds the new rows; old scope select, CLI details popover and project writes removed.
- [ ] fmt, clippy, targeted GPUI tests pass.
