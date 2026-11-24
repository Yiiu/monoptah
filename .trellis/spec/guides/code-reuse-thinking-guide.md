# Code Reuse Thinking Guide

> **Purpose**: Stop and think before creating new code - does it already exist?

---

## The Problem

**Duplicated code is the #1 source of inconsistency bugs.**

When you copy-paste or rewrite existing logic:
- Bug fixes don't propagate
- Behavior diverges over time
- Codebase becomes harder to understand

---

## Before Writing New Code

### Step 1: Search First

```bash
# Search for similar function names
grep -r "functionName" .

# Search for similar logic
grep -r "keyword" .
```

### Step 2: Ask These Questions

| Question | If Yes... |
|----------|-----------|
| Does a similar function exist? | Use or extend it |
| Is this pattern used elsewhere? | Follow the existing pattern |
| Could this be a shared utility? | Create it in the right place |
| Am I copying code from another file? | **STOP** - extract to shared |

---

## Common Duplication Patterns

### Pattern 1: Copy-Paste Functions

**Bad**: Copying a validation function to another file

**Good**: Extract to shared utilities, import where needed

### Pattern 2: Similar Components

**Bad**: Creating a new component that's 80% similar to existing

**Good**: Extend existing component with props/variants

### Pattern 3: Repeated Constants

**Bad**: Defining the same constant in multiple files

**Good**: Single source of truth, import everywhere

### Pattern 4: Repeated Payload Field Extraction

**Bad**: Multiple consumers cast the same JSON/event fields locally:

```typescript
const description = (ev as { description?: string }).description;
const context = (ev as { context?: ContextEntry[] }).context;
```

This is duplicated contract logic even when the code is only two lines. Each
consumer now has its own definition of what a valid payload means.

**Good**: Put the decoder, type guard, or projection next to the data owner:

```typescript
if (isThreadEvent(ev)) {
  renderThreadEvent(ev);
}
```

**Rule**: If the same untyped payload field is read in 2+ places, create a
shared type guard / normalizer / projection before adding a third reader.

---

## When to Abstract

**Abstract when**:
- Same code appears 3+ times
- Logic is complex enough to have bugs
- Multiple people might need this

**Don't abstract when**:
- Only used once
- Trivial one-liner
- Abstraction would be more complex than duplication

---

## After Batch Modifications

When you've made similar changes to multiple files:

1. **Review**: Did you catch all instances?
2. **Search**: Run grep to find any missed
3. **Consider**: Should this be abstracted?

### Reducers Should Use Exhaustive Structure

When state is derived from action-like values (`action`, `kind`, `status`,
`phase`), prefer a reducer with one `switch` over scattered `if/else` updates.

```typescript
// BAD - action-specific state transitions are hard to audit
if (action === "opened") { ... }
else if (action === "comment") { ... }
else if (action === "status") { ... }

// GOOD - one reducer owns the transition table
switch (event.action) {
  case "opened":
    ...
    return;
  case "comment":
    ...
    return;
}
```

This matters when the event log is the source of truth. A reducer is the
documented replay model; display code and commands should not duplicate pieces
of that replay model.

---

## Checklist Before Commit

- [ ] Searched for existing similar code
- [ ] No copy-pasted logic that should be shared
- [ ] No repeated untyped payload field extraction outside a shared decoder
- [ ] Constants defined in one place
- [ ] Similar patterns follow same structure
- [ ] Reducer/action transitions live in one reducer or command dispatcher

---

## Shared Pieces in This Repository

Fix behavior in the shared piece so every caller gets it (`CLAUDE.md`, "Fix at the root, in
the shared component": the first fix for the dialog input barrier went into individual
dialogs and reached 2 of about 35). Search for these before writing a new one:

| Need | Reuse | Spec |
|---|---|---|
| Native dialog, overlay, input barrier | `crates/monocode-gpui/src/modal.rs` | [native/component-guidelines.md](../native/component-guidelines.md) |
| Native menu, dropdown, choice select | `crates/monocode-gpui/src/popover.rs` | [native/component-guidelines.md](../native/component-guidelines.md) |
| Native colors | `crates/monocode-gpui/src/theme.rs` | [native/component-guidelines.md](../native/component-guidelines.md) |
| Running a sandbox (`sbx`) command | the one runner in `crates/monocode-core/src/sandboxes.rs` | [core/processes-and-platforms.md](../core/processes-and-platforms.md) |
| Writing a file safely | `write_atomic` in `crates/monocode-core/src/mono/files.rs` | [core/persistence-and-compat.md](../core/persistence-and-compat.md) |
| React UI primitives and hooks | `src/shared/` | [frontend/component-guidelines.md](../frontend/component-guidelines.md) |

Logic that is not drawing (parsing, storage, process control) belongs in `monocode-core`,
not in a native view, so it can be tested without a window and shared across views.
