# Development Workflow Specification

## Purpose

Keep Monoptah's OpenSpec workflow reproducible across coding tools and sessions
while preserving the repository's established engineering and verification rules.

## Requirements

### Requirement: Reproducible project-local CLI

The repository SHALL pin an exact OpenSpec CLI development dependency and commit
its npm lockfile. The CLI SHALL be available from the repository root through
`npm run openspec -- <arguments>`.

#### Scenario: Use the CLI after installing dependencies

- **WHEN** a contributor installs the locked dependencies using a supported Node.js version
- **THEN** `npm run openspec -- --version` reports the project-pinned OpenSpec version
- **AND** no global OpenSpec installation is required

### Requirement: Shared project-scoped skills

The repository SHALL provide OpenSpec core workflow skills for Claude Code and
Codex in project scope, with shared content under `.agents/skills/` and Claude
Code links under `.claude/skills/`.

#### Scenario: Initialize or repair the project integration

- **WHEN** a contributor runs `npm run openspec:init` from the repository root
- **THEN** OpenSpec configures the core workflow skills for Claude Code and Codex in this project
- **AND** existing project context and authored specifications are preserved

### Requirement: Isolate project preferences and skill refresh

Project npm commands SHALL keep OpenSpec CLI preferences and remembered projects
inside the ignored `.openspec-local/` directory. Project skill refresh SHALL use
`npm run openspec:init` and SHALL NOT refresh machine-wide skills. The launcher
SHALL reject the upstream `update` command and initialization with global scope.

#### Scenario: A contributor has a different machine-wide profile

- **WHEN** a contributor initializes or refreshes this project's OpenSpec skills
- **THEN** the configured project core skills are generated
- **AND** machine-wide OpenSpec preferences remain unchanged

#### Scenario: A contributor requests upstream update through the project launcher

- **WHEN** a contributor runs `npm run openspec -- update`
- **THEN** the launcher rejects the command before starting OpenSpec
- **AND** it directs the contributor to `npm run openspec:init`

### Requirement: Preserve established engineering rules

OpenSpec planning and implementation SHALL follow `CLAUDE.md` and
`docs/git-conventions.md`. Project configuration SHALL expose those constraints
to artifact generation and implementation without replacing their authoritative
source files.

#### Scenario: Plan and implement a native change

- **WHEN** an OpenSpec change affects the native Rust application
- **THEN** the plan and implementation use the `native-dev` build profile
- **AND** verification follows the repository's batched checks and review rules

### Requirement: Distinguish specification validation from implementation verification

The repository SHALL provide `npm run openspec:check` to validate all active
OpenSpec specifications and changes in strict, non-interactive mode. A successful
specification validation SHALL NOT be treated as proof of correct implementation.

#### Scenario: Finish a documented change

- **WHEN** a contributor validates the OpenSpec artifacts before archiving a completed change
- **THEN** the artifacts pass structural validation
- **AND** implementation verification and review outcomes are recorded separately
