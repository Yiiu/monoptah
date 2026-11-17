# Native large-tool write bound follow-up

Date: 2026-11-17. Follow-up to the authorized native transcript write-amplification
spec and the user's instruction to continue optimizing the remaining large-tool
bottleneck. Implementation choices below are the implementer's calls, not new
user-approved requirements. Existing safety fixes remain in this checkout.

## Intent and constraints

A small append to a 5 MiB tool input must not rewrite its duplicated titles/text in
`native_blocks.skeleton_json`. Preserve exact block values, including hidden and
unknown metadata, normal save cadence, transaction/receipt boundaries and SQLite
durability. Work on native core only; use synthetic temporary databases. No commits,
pushes, releases, production conversion or changes to production app data.

The existing renderer copies raw non-JSON tool input into top-level `text`,
`tool.title` and `tool.preview.title`, as well as retaining `tool.nativeInput`.
Stripping just selected input members leaves multiple giant strings inline. The
current writer then upserts the whole skeleton, while chunk counters omit it.

## Approach

Use the existing segmented payload representation for oversized JSON string values
wherever they occur in a block, including nested objects/arrays and unknown fields.
Preserve the root block ID inline because the writer uses it without reconstructing
payloads. Plain text that is exactly recoverable from rows keeps the existing
text sentinel. Small blocks keep their current raw representation.

Changing displayed captions would change stored/user-visible content. A separate
skeleton table or a new marker is unnecessary: existing mode-1 readers substitute
payload placeholders before parsing, independently of their JSON location. Keep
mode version 1 and test both legacy and newly stripped representations. This is a
bound for giant string payloads, not a universal bound on arbitrary huge JSON keys
or objects composed entirely of many small values. The unchanged probe starts with
small captions: its first input update creates full-sized title/text copies, which
are real new data and remain in the hot-stage total. The bound applies to later
incremental updates of those existing strings. Very large `ToolCall.display` row
metadata can have separate amplification; the targeted steady-state test uses a
small explicit `nativePresentation` and does not establish a bound for that case.

Legacy mode-1 data must remain readable and exportable before its next save.
Downgrade's exact stored-form verification must accept both the original canonical
member selection and the expanded selection, while retaining projection, revision,
transaction and content-equality checks. Old readers can reconstruct the new
representation; old strict exporters may refuse a form their writer would not
produce. Use the updated export implementation before downgrading.

Add `skeleton_bytes_written` without changing `chunk_bytes_written`: the sum is
committed transcript JSON bytes submitted by the native writer. It excludes scalar
metadata/index/page/WAL/filesystem overhead; keep block/chunk counts and OS counters
separate. `serialized_bytes` measures encoding work and is not a committed-write
counter. Failed transactions must not publish committed-write counters.

The probe/comparator gain schema version 3 with required skeleton counters and
backend-specific null reasons. Preserve fixtures/workloads. Historical v2 evidence
is retained; new comparisons require paired v3 producers. Reject mixed schemas or
invented missing counters rather than filling them with zero.

## Verification and limits

Batch meaningful regression edits, prove the giant skeleton/accounting failure,
then apply the layout/legacy-export fix. Cover nested/unknown/Unicode strings,
marker refusal, exact round trips, old-layout downgrade and save, bounded combined
JSON writes, no-change zero writes, and failed-commit counters.

After measurement-tool checks, retain one instrumented pre-layout large-tool run
and executable; then run the same workload after the layout fix. Windows reports
functional/logical results only: raw reports remain incomplete when process I/O is
unavailable, and no 95% OS-write or UI-latency acceptance is inferred. Preserve all
attempts and unknown costs. macOS matched process-I/O measurement and clean-origin
Native Dev remain pending. One whole-request code review covers the final diff,
with particular attention to legacy-format export safety.
