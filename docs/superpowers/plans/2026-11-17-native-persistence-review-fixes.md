# Native persistence review fix ledger

Base: `139f600013c2e9d9e2d7d2e4d56c76bd24595163`.
Spec: `docs/superpowers/specs/2026-11-16-native-transcript-write-amplification-design.md`.

The user authorized continuing the persistence work on 2026-11-17. The original
commit records twelve adjudicated review findings, but its ignored review artifacts
are absent from this checkout and neither fork has an associated pull request.
This ledger tracks independently verified fixes; it does not reconstruct the seven
unrecorded findings from guesses.

Ruling: continue in this clean, dedicated `worktree-native-persistence-safety`
checkout rather than create a second copy — this is a resumed branch and there is
no other local worktree or native build cache — the cost is that the changes stay
uncommitted on the user's existing task branch.

Ruling: follow the repository's single verification pass after the batch, overriding
skills' per-edit test runs — add the regression tests together, run them against
the original implementation, then apply fixes and run related checks together —
individual failures must still be diagnosed and reported.

## Scope and hypotheses

- [x] Cross-handle mode changes: a handle loaded in one representation must refuse
  writes after conversion/downgrade by another handle, until a successful reload.
  Cover sparse writes and the public full-block replacement.
- [x] Future format versions: full-history and paged readers must refuse unsupported
  versions instead of exposing an empty or incorrectly decoded conversation;
  supported sessions must remain readable when another session is unsupported.
- [x] Backup retention: keep the previous verified recovery point until its complete,
  verified replacement is ready; retain at most one published backup per session.
- [x] Public `store_blocks`: store the transcript and list flags atomically, including
  failure/rollback, without leaving a revision cache that prevents retry.
- [x] Receipt scan: do not re-read a draft after deleting it in the same acceptance
  transaction; preserve queue/receipt atomicity.

## Reproductions and implementation

The original implementation failed eight regression cases in the first run
(exit 101; 51 passed / 8 failed, including unrelated names matched by `review_`).
After the first fixes, the full native-chunks module ran 33 tests: 32 passed,
including all eight original failures, and the newly added immediate-post-conversion
save test failed because the conversion cached revision 0 instead of its committed
revision.

A second scoped reproduction then confirmed three more recovery boundaries:
reconversion reused a stale native revision, and failed BUSY commits left both
sparse-save and downgrade transactions open. All four failures pass in the final
validation below.

Ruling: use a fresh 62-bit random revision starting point on each conversion and
return the committed revision in `ConversionReport` — mode alone does not detect
native→blob→native transitions, and resetting revisions to zero permits stale
writes after reconversion — collision probability is nonzero (2^-62 per pair),
while no schema change or persistent generation table is added.

Ruling: preserve one latest verified backup per session via temporary-file
verification and atomic replacement, syncing file contents and the Unix parent
directory — the prior recovery point must survive failed copies, and unlimited
whole-database backups would grow without bound — successfully replacing it drops
the older recovery snapshot for that session, so this is not a historical archive.

## Independent review and single fix round

The reviewer returned **not ready** and proved five assertions fail using temporary
executables compiled against the current core rlib (no source changes): stale
content is acknowledged by conversion/downgrade, the public blocks read/write pair
has no guard, concurrent downgrade clears an already-exported history, and accepting
the sole draft cannot commit. A backup publication race was also identified by a
concrete interleaving. These are grouped as five findings; none may be treated as
covered by the earlier 450 passing tests.

The single review fix round added seven synthetic regressions. Before those fixes,
the native-chunks run passed 36/43 and failed exactly these seven cases (exit 101).
The backup test uses a test-only callback after copying/verifying a temporary
snapshot to let another handle finish a newer conversion before the first resumes.
Conversion now validates the prior loaded blob inside its write transaction;
downgrade rechecks its mode/revision there and returns a blob guard; the public
block reader establishes a matching snapshot/mode guard. Deleted drafts no longer
determine append sequence numbers. Only a transaction that still matches its
source snapshot may publish its prepared backup.

All five review findings were fixed in this round and their seven regressions pass:

1. **Stale content acknowledged by conversion/downgrade:**
   `crates/monocode-core/src/store/native_chunks/convert.rs:232` validates the
   prior blob hash while holding the write lock; `:328` returns the exported blob
   hash to retain the same handle's guard. Covered by
   `review_conversion_refuses_a_stale_loaded_blob` and
   `review_downgrade_keeps_a_blob_guard_for_the_same_handle`.
2. **Public blocks read/write guard bypass:**
   `crates/monocode-core/src/store/shared.rs:1129` reads content, mode and revision
   from one snapshot; `:1272` checks the guard inside the replacement transaction.
   Covered by `review_stored_blocks_guards_blob_to_native_replace` and
   `review_stored_blocks_guards_native_to_blob_replace`.
3. **Concurrent double downgrade exports an empty history:**
   `crates/monocode-core/src/store/native_chunks/convert.rs:341` rechecks the mode
   and expected revision under the write lock before reading native content.
   Covered by `review_downgrade_rechecks_mode_inside_the_transaction`.
4. **Acceptance cannot replace the sole draft:**
   `crates/monocode-core/src/store/shared.rs:1961` bases append IDs and row offsets
   on the surviving tail after draft deletion. Covered by
   `review_acceptance_of_the_only_draft_can_commit_and_retry`, including rollback
   and successful retry.
5. **Late backup replaces a newer recovery point:**
   `crates/monocode-core/src/store/native_chunks/convert.rs:539` only prepares the
   backup; `:286` publishes it after source validation under `BEGIN IMMEDIATE`.
   Covered by `review_late_backup_cannot_replace_a_newer_conversion_recovery_point`.

The reviewer was not asked for a second whole-request review; the outcomes above
are established by the reproduction/fix tests in the final run.

Review declined to judge: the seven missing historical findings, large-tool storage
and measurement completeness, live Native Dev/macOS measurements, and unrelated
old cloud/TUI/Mono indexed behavior. Ruling: retain these as explicit outstanding
scope rather than claim them closed — this batch changes ordinary local safety
only — the cost is that the overall persistence rollout remains unfinished.

## Validation

Final combined validation completed on 2026-11-17 with **exit 0**, using Rust
1.97.1 and the `native-dev` profile on Windows:

- Core clippy (`-p monocode-core --all-targets -- -D warnings`) passed.
- Core tests filtered by `store::`: **457 passed, 0 failed**. This includes all
  **43 native-chunks tests** and all **20 added safety regressions**.
- `transcript_save_probe` example tests: **19 passed, 0 failed**.
- A final `cargo fmt --all -- --check` and `git diff --check` passed.

Commands (each exit status was checked directly, without a pipe):

```text
cargo +1.97.1 clippy --locked --manifest-path crates/monocode-gpui/Cargo.toml --profile native-dev -p monocode-core --all-targets -- -D warnings
cargo +1.97.1 test --locked --manifest-path crates/monocode-gpui/Cargo.toml --profile native-dev -p monocode-core store::
cargo +1.97.1 test --locked --manifest-path crates/monocode-gpui/Cargo.toml --profile native-dev -p monocode-core --example transcript_save_probe
cargo +1.97.1 fmt --manifest-path crates/monocode-gpui/Cargo.toml --all -- --check
git diff --check
```

Before the independent review, the smaller batch passed 450 store tests and 19
probe tests. Core clippy found the pre-existing `next_append_seq += 1` loop-counter
lint; assigning the next sequence from the validated appended block preserves the
append-only behavior and passed both the targeted rerun and final clippy.
The probe test build still emits its existing `unused_mut` warning at
`examples/transcript_save_probe.rs:1750`; the dependency build emits MSVC linker
output warnings. Neither failed the checks above.

The first build completed in 10m48s after disabling HTTP multiplexing for dependency
downloads. No production databases, provider calls, repository commits, pushes,
releases or production conversions have been performed. Changes remain uncommitted
on the dedicated task branch. The full native gate and live application verification
were not run for this core-only safety task.

The large-tool source diagnosis identified giant `tool.title`/`text` members left
in `native_blocks.skeleton_json`, plus omission of skeleton bytes from the chunk
counter. The profiling document now marks large-message acceptance unmet and
records the call chain. This is a source diagnosis, not a new process-I/O benchmark;
no layout or UI behavior was changed in this safety batch.

Live Native Dev verification and the large-tool process-I/O diagnosis remain later
steps. Historical macOS measurements in `docs/native/persistence-profiling.md` are
retained evidence, not measurements made on this Windows machine.

## User-requested verification (2026-11-17)

Repeated the related checks on request: **457/457 store tests**, **19/19 probe
example tests**, core clippy with `-D warnings`, formatting and the runnable probe
build passed (combined exit 0).

Actual probe invocation then exposed a Windows tooling defect. `build.rs` expanded
1,142 source paths into a 52,952-character `git diff` invocation; process creation
failed with `ENAMETOOLONG`. The embedded identity had `complete: false`, so all six
initial attempts correctly refused before database setup and sampled zero saves.
Their raw reports and unknown I/O costs remain retained.

Fixed `crates/monocode-core/build.rs:76` to pass the fixed directory/file scope to
`git diff`, retaining individual source-content hashing. Ruling: conservatively
include tracked metadata changes within that scope in the patch hash — this keeps
the command bounded on Windows — the patch-hash scope is broader, so any future
performance pair must use the repaired producer on both sides. Historical evidence
is unchanged. After this tool-only fix, core clippy, all 19 probe tests and the
probe build passed again; storage implementation files did not change.

The rebuilt executable's embedded identity is complete and its source-tree hash
matches an independent hash of the current checkout:

- Source-tree SHA256: `6e7c9e8d61ca1c17495fa5d3991f94685b5953d58d575493002dbc339c074ed7`.
- Executable SHA256: `60c1016efeb846150212d51c666221687ef77a50462f905726d36fa8b3c79879`.

Six actual workloads completed **42 successful saves**: blob `append-1m` (5), native
`append-1m` (5), `same-value` (5), `early-tool` (2), `large-tool` (5), and `parallel`
(20 across four sessions). All eight integrity flags were true for every run:
initial fixture, rows, completed history, unknown metadata, receipts, queue,
close/reopen equality and SQLite integrity. Every declared event was sampled,
all six functional stages completed, and each final checkpoint was non-BUSY with
matching frame counts. The blob/native append-1m manifests and saved/reopened
content fingerprints also matched.

The functional verifier exited 0. Every raw probe report still exits **2**, is
`incomplete`, and retains the sole reason `process_io_unavailable`: Windows has no
implementation of the macOS process-write counter. This does not establish the
95% target, UI responsiveness or large-tool performance acceptance. Across the six
initial and six repaired attempts, all twelve process-I/O counters are unknown;
`experiment-cost.json` therefore keeps `total_bytes: null`.

Local raw artifacts, manifests, invocations, stderr and summaries are retained at:

```text
C:/Users/a1103/AppData/Local/Temp/monoptah-persistence-verification-20261117-b14nmr4zl/
  artifacts/                 # six pre-setup refusals
  artifacts-identity-fixed/  # six completed functional workloads; summary.json
```

The first temporary verifier compared two null fingerprints in its parity check;
that initial parity result is invalid and excluded. The repaired verifier requires
non-null manifests and saved/reopened fingerprints before comparing them.

The installed app's read-only `terminals.list` returned no project terminals.
Native Dev GUI verification remains pending: `CLAUDE.md:83` requires a clean
checkout matching origin, and these fixes are still uncommitted. Production data
and credentials were not used by the synthetic workloads.
