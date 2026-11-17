# OpenSpec in Monoptah

OpenSpec keeps a change's requirements, design, task list and history in Git so
work can continue across sessions and coding tools. Engineering rules remain in
[CLAUDE.md](../CLAUDE.md) and [Git conventions](git-conventions.md).

## Setup

The project pins `@fission-ai/openspec` 2.1.0 as a development dependency. Its npm
CLI requires Node.js 22.20.0 or newer; the active native CI uses Node.js 22.

```bash
npm ci
npm run openspec -- --version
```

Project skills are committed in `.agents/skills/`, with relative links under
`.claude/skills/`. Codex reads `.agents/skills/`; Claude Code reads
`.claude/skills/`. After cloning, run the initializer if your checkout did not
preserve the links or your coding tool cannot discover the skills:

```bash
npm run openspec:init
```

The initializer selects the core profile, Claude Code and Codex, and project
scope. Artifact language is set in `openspec/config.yaml`. Restart the coding
session after installing or refreshing skills. Use the CLI from the repository root:

```bash
npm run openspec -- list --json
```

When a generated skill or CLI hint says `openspec ...`, use
`npm run openspec -- ...` instead. This keeps commands on the lockfile's version
without requiring a global installation. The launcher also keeps CLI preferences
and remembered projects under the ignored `.openspec-local/` directory, selects
project skill scope, and disables CLI telemetry for these commands.

OpenSpec 2.1's `update` command can refresh machine-wide skills. The launcher
rejects that command; use `npm run openspec:init` to refresh project skills instead.
It also rejects initialization with `--scope global`.

## When to use it

Use OpenSpec for new behavior, changes spanning multiple modules, and changes to
compatibility or persistence. Small repairs and documentation updates can follow
the normal repository workflow unless OpenSpec was explicitly requested.

The initial `development-workflow` spec describes this integration. Existing app
behavior is documented incrementally as capabilities are changed; the empty or
missing specification of a legacy capability is not permission to redesign it.

## Working on a change

Start with an idea in the coding assistant. For example, in Claude Code:

```text
/openspec-explore investigate how session recovery currently works
/openspec-propose improve-session-recovery
```

Codex uses `$openspec-explore` and `$openspec-propose`; in its desktop app, select
the corresponding skill. Natural-language requests naming the OpenSpec workflow
also work when the assistant supports skill discovery.

`explore` is optional. `propose` creates the planning artifacts in
`openspec/changes/<change-name>/`:

- `proposal.md`: why the change is needed, its scope and affected capabilities.
- `specs/<capability>/spec.md`: the required behavior added, modified or removed.
- `design.md`: the technical approach when the schema calls for a design.
- `tasks.md`: implementation and verification checkboxes.

Review the proposal and requirements before asking the assistant to apply the
change. In Claude Code:

```text
/openspec-apply-change improve-session-recovery
```

Use `openspec-update-change` to revise an existing plan when decisions change.
Supporting development skills use these artifacts as the plan and task record;
keep one review per request and follow the repository's native testing rules.

Inspect progress and validate specification structure in the terminal:

```bash
npm run openspec -- status --change improve-session-recovery
npm run openspec:check
```

OpenSpec validation checks specification structure. It does not prove that the
implementation is correct. Record the relevant tests, checks and review results;
native work uses `native-dev` and the batching rules in CLAUDE.md.

After verification, request `openspec-archive-change` for the completed change.
The archive workflow checks task completion, assesses changes to main specs, and
archives the change under `openspec/changes/archive/`. Archiving does not merge a
branch or publish a release. `openspec-sync-specs` is also available when main
specs need updating before archive.

## Project structure

- `openspec/config.yaml`: project context, artifact rules and apply/archive guidance.
- `openspec/specs/`: the current documented capabilities.
- `openspec/changes/`: work in progress, with completed changes in `archive/`.
- `.agents/skills/openspec-*`: generated, shared workflow skills.
- `.claude/skills/openspec-*`: generated links to those shared skills.
- `AGENTS.md`: directs other coding agents to the shared CLAUDE.md rules.

Commit the configuration, authored specs and changes, generated skills and links,
and dependency lockfile together when updating this integration. Keep generated
skill content unmodified; put project constraints in `config.yaml` and the shared
engineering instructions.

## Updating the CLI and skills

Choose a new version deliberately and refresh generated skills with that version:

```bash
npm install --save-dev --save-exact @fission-ai/openspec@<version>
npm run openspec:init
npm run test:openspec
npm run openspec:check
```

Review generated changes and retain project context and authored specifications.
The project does not change global OpenSpec profiles or install global skills.

Official documentation: <https://openspec.dev/docs>.
