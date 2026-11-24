# Cross-Layer Thinking Guide

> **Purpose**: Think through data flow across layers before implementing.

---

## The Problem

**Most bugs happen at layer boundaries**, not within layers.

Common cross-layer bugs:

- API returns format A, frontend expects format B
- Database stores X, service transforms to Y, but loses data
- Multiple layers implement the same logic differently

---

## Before Implementing Cross-Layer Features

### Step 1: Map the Data Flow

Draw out how data moves:

```
Source → Transform → Store → Retrieve → Transform → Display
```

For each arrow, ask:

- What format is the data in?
- What could go wrong?
- Who is responsible for validation?

### Step 2: Identify Boundaries

| Boundary              | Common Issues                     |
| --------------------- | --------------------------------- |
| API ↔ Service         | Type mismatches, missing fields   |
| Service ↔ Database    | Format conversions, null handling |
| Backend ↔ Frontend    | Serialization, date formats       |
| Component ↔ Component | Props shape changes               |

### Step 3: Define Contracts

For each boundary:

- What is the exact input format?
- What is the exact output format?
- What errors can occur?

---

## Common Cross-Layer Mistakes

### Mistake 1: Implicit Format Assumptions

**Bad**: Assuming date format without checking

**Good**: Explicit format conversion at boundaries

### Mistake 2: Scattered Validation

**Bad**: Validating the same thing in multiple layers

**Good**: Validate once at the entry point

### Mistake 3: Leaky Abstractions

**Bad**: Component knows about database schema

**Good**: Each layer only knows its neighbors

### Mistake 4: Every Consumer Parses The Same Payload

**Bad**: A command reads JSONL events and casts fields inline:

```typescript
const thread = (ev as { thread?: string }).thread;
const labels = (ev as { labels?: string[] }).labels;
```

This looks local, but it means every consumer owns a private version of the
event contract. The next field change will update one command and miss another.

**Good**: Decode once at the event boundary, then export typed projections:

```typescript
if (!isThreadEvent(ev)) return false;
return ev.thread === filter.thread;
```

**Rule**: For append-only logs, JSON streams, RPC payloads, or config files,
create one owner for:

- event / payload type definitions
- type guards and normalization from `unknown`
- metadata projections used by UI commands
- reducers that replay state from the source of truth

Rendering code may format fields, but it must not redefine the payload contract.

---

## Checklist for Cross-Layer Features

Before implementation:

- [ ] Mapped the complete data flow
- [ ] Identified all layer boundaries
- [ ] Defined format at each boundary
- [ ] Decided where validation happens

After implementation:

- [ ] Tested with edge cases (null, empty, invalid)
- [ ] Verified error handling at each boundary
- [ ] Checked data survives round-trip
- [ ] Checked that consumers import shared decoders / projections instead of
      casting payload fields locally
- [ ] Checked that derived state points back to the source event identifier
      (`seq`, `id`, `version`) instead of inventing a second cursor

---

## Boundaries in This Repository

These are the layer boundaries where Monoptah changes most often break. Each one has an
owner spec; read it before changing either side.

| Boundary | What crosses it | Owner spec |
|---|---|---|
| React UI (`src/`) ↔ Tauri backend (`src-tauri/`) | Tauri commands, their serde argument and return shapes, emitted events | [frontend/platform-bridge.md](../frontend/platform-bridge.md), [tauri/commands-and-events.md](../tauri/commands-and-events.md) |
| Native UI (`crates/monocode-gpui`) ↔ `crates/monocode-core` | Domain services, `Store`, provider turns, process control. No GPUI types in core | [native/directory-structure.md](../native/directory-structure.md), [core/directory-structure.md](../core/directory-structure.md) |
| Native ↔ the installed desktop app's data | The shared `monocode.db`, its columns and migration ledger, sidecar files | [core/persistence-and-compat.md](../core/persistence-and-compat.md) |
| Apps ↔ remote Host (`host/`) | The Host protocol, `host/VERSION`, release assets whose sha256 the app pins | [host/protocol-and-compat.md](../host/protocol-and-compat.md) |
| Desktop ↔ mobile (`mobile/`, `crates/mobile-link`) | The pairing and sync protocol | [mobile/index.md](../mobile/index.md), [tauri/directory-structure.md](../tauri/directory-structure.md) |

### Data that outlives the code

Installed apps keep data written by older builds, so a stored value is a contract with code
you cannot update. Before renaming or reshaping anything persisted, check:

- [ ] Is it one of the identifiers in `CLAUDE.md`, "Never rename these" (the `monocode.*`
      localStorage prefix, `monocode-host` names, `INTERRUPT_MESSAGE`, package and crate
      names)? Then it stays, even when the surrounding copy is rebranded.
- [ ] Can an older build still read what the new build writes, and can the new build read
      old rows? New columns and fields need defaults (`#[serde(default)]`, nullable SQL
      columns).
- [ ] Does a fixture compare bytes? Fixtures listed in `.gitattributes` with `eol=lf` must
      not be rewritten with CRLF.

### Ported behavior

Native ports features from the React app, and many native modules name the React file they
port (see [native/index.md](../native/index.md)). When you change behavior that exists in
both apps, decide explicitly whether the other app changes too, and say so in the PR.

### Upstream merges

Upstream (`hardbeat920/monocode`) keeps adding copy and links that say "MonoCode". Any
cross-layer change that adds user-visible text or links must pass
`./scripts/check-fork-branding.sh` (see `CLAUDE.md`, "After every merge from upstream").
