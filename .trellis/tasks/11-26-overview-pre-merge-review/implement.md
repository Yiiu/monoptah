# Implement: pre-merge review

Read-only first. Work in a worktree of `feat/agent-overview-tree`, as described in the parent `handoff.md`.

1. Read the parent `prd.md`, `research/audit-status-mono.md` and `research/audit-tree-summary.md`, so known items are not re-reported.
2. Run `git diff main...feat/agent-overview-tree --stat`, then read the four focus areas in `prd.md` "Scope" in this order:
   1. migration
   2. data loss
   3. timers
   4. Mono
3. For every suspected defect, write a concrete failure scenario. Build or run a test only to prove a specific finding:
   - `cargo +1.98.1 ntest -p monocode-core <filter> -- --test-threads=1`
   - `cargo +1.98.1 ntest -p monocode-gpui --bin monocode-gpui <filter> -- --test-threads=1`
4. Write `research/review-findings.md` in the format given in `prd.md` "Output".
5. Fix small isolated blockers directly. Give each its own commit (`fix(native): ...` with a body) and a test that fails without the fix.
6. Correct the stale docs: branch name and CI status in `11-25-agent-overview/prd.md` and `11-25-agent-status-core/prd.md`. Commit them as `docs(trellis): ...`.
7. Push `feat/agent-overview-tree`. In the PR #22 conversation, post a short comment that links the findings file and lists the blockers.

Gate: no blocker may be left unassigned.
