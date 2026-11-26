# Persistence and Compatibility

> Native shares its data with the installed desktop (Tauri) app and with older Native builds.
> Every stored byte is a contract with code you cannot update. Read this before changing any
> column, field name, file format or identifier.

---

## The session database belongs to the desktop app

- Production Native opens the **desktop app's** database, `monocode.db` under
  `<data dir>/com.monoptah.desktop/` (`src/store/monocode_import.rs::monocode_database`:
  `~/Library/Application Support` on macOS, `%APPDATA%` on Windows, `$XDG_DATA_HOME` or
  `~/.local/share` on Linux), through `Store::open_monocode` (`src/store/shared.rs`).
- `src/store/shared_schema.sql` starts with "New installations only. Existing databases are
  validated, never altered." It is applied only when the file did not exist. For an existing
  file, `open_monocode` checks required `sessions` columns and refuses with
  `"Unsupported Monoptah database: missing sessions.{required}. No schema changes were made."`
  The `schema_migrations` row (`version 19`) is the desktop app's migration ledger: do not add
  rows from Native.
- Native-only data never goes into desktop tables:
  - key/value metadata lives in a sidecar SQLite file `<db>.gpui.sqlite3`, migrated from the
    older `<db>.gpui.json` (`src/store/shared/settings.rs`);
  - the chunked transcript backend (`native_sessions` marker + `native_blocks`,
    `native_chunks`, `native_payloads`) is created only in a brand-new database
    (`ensure_native_chunks_tables` inside the new-file branch of `open_monocode`).
- `Store::open` (`src/store.rs`) is the legacy prototype schema (`native_sessions`,
  `native_rows`, ...), kept for fixtures. It is the one place that still adds columns at open,
  idempotently: read `PRAGMA table_info`, then `ALTER TABLE ... ADD COLUMN` only for missing
  names. `src/store/notes.rs::ensure_notes_table` follows the same check-then-add pattern. New
  columns must be nullable or carry a `DEFAULT` so old rows and old builds keep working.

## Forward compatibility: refuse, never guess

- `src/store/native_chunks/mod.rs`: `NATIVE_CHUNKS_MODE_VERSION` (2) is the newest storage
  mode this build writes. A marker with a higher version makes the whole connection refuse writes
  with the `UnsupportedStorageMode` kind: "An unrecognized format must refuse writes, never read
  as an empty history." If you add a storage mode, bump the version and keep this barrier.
- `src/mono/workflows/store.rs`: entries this app cannot read "are kept as they were and written
  after the others, and a file that is not a JSON list is never overwritten"
  (`LoadedWorkflows::unreadable`). Apply the same rule to any list file shared with another app.

## Native conversation history previews

- **Scope.** Use `Store::chunked_history_window(session, anchor, turns)` for the first
  paint of ordinary local conversations stored in Native chunks. Indexed Mono, remote, and
  legacy blob histories use their existing readers.
- **Signature.** The method returns `Result<Option<(usize, Vec<Row>)>, String>`.
  The tuple contains the absolute index of its first row and the rows in display order.
- **Contract.** `None` as the anchor requests the latest turns. An absolute row anchor
  requests a window around that row. Prompt boundaries and rows come from one read snapshot.
  Keep the absolute origin when mapping a displayed row to search or scroll positions.
- **Fallback.** `Ok(None)` means a preview is unavailable, including when a saved anchor
  is outside the current history. `Err` reports a failed read. Conversation loading still runs
  the authoritative reader in either case.
- **Examples.** A new view shows the latest turns, an evicted view opens around its saved
  row, and an invalid anchor falls back to the authoritative history. The preview's lifetime
  is limited to loading. Sends, edits, and retries depend on the loaded history and its
  revision check.
- **Tests.** `chunked_preview_uses_absolute_turn_boundaries_and_keeps_full_history` asserts
  absolute offsets, legacy fallback and unchanged revisions.
  `reading_an_early_anchor_skips_unrelated_later_chunk_payloads` guards payload reads.
  `an_evicted_local_chat_opens_on_its_old_reading_position` verifies scroll restoration.
- **Read path.** Keep `agent_settings` on the prompt-source metadata path for ordinary
  Native chunks. Reconstructing all history blocks there repeats payload work before
  `Store::load`. The full reader supplies the history and write revision.

## Writing files and databases

- Atomic file writes: write a sibling temp file and `rename` over the target
  (`src/mono/files.rs::write_atomic`; also `src/checkpoint.rs`, `src/accounts/token.rs`).
  Never truncate the real file in place.
- Optimistic concurrency for shared files: Mono files are saved against the content hash that
  was read (`mono::files::content_hash`, sha256 hex). A lost race returns `files::CONFLICT`;
  callers reread and re-apply, `ATTEMPTS = 3` in `src/mono/workflows/store.rs`.
- Owner-only permissions on Unix: databases and their `-wal`/`-shm` get `0o600`
  (`src/store.rs::restrict_database_files`); new private files are opened with
  `OpenOptionsExt::mode(0o600)` (`write_atomic`, `store/shared/settings.rs`).
- SQLite connections set `busy_timeout(5s)` and `PRAGMA foreign_keys=ON`; WAL is set only when
  creating a file. Multi-step writes use a transaction (`store.rs::upsert_rows` is an idempotent
  `INSERT ... ON CONFLICT DO UPDATE`).
- Secrets: Claude credentials are AES-256-GCM sealed in `provider_account_credentials`; the key
  lives in a separate file next to the database, "never in the database itself"
  (`src/accounts/token.rs`). OS keychains are used via `keyring` per platform (see
  `Cargo.toml` target sections). Never store a token in plain JSON settings.
- Documented exception: per-agent environment variables (`preferences.agents.env`, the Agent
  CLIs settings page) are stored as plain text in the native workspace JSON, by product
  decision (2026-11-24). Mitigations: values whose name contains KEY/TOKEN/SECRET/PASSWORD/AUTH
  are masked on screen, and env values never go into logs, aria labels, debug selectors,
  toasts or error text. Moving them to the OS credential store is a follow-up; do not add other
  plain-text secrets on the strength of this exception.
- The desktop app shares `sessions.runtime_mode`: never write a value it does not know. Native's
  "agent's own settings" access is written there as its peer id (`AccessMode::peer_id`) and
  kept exactly in the native sidecar record `agent:{id}` (`store/shared.rs`).

## Serde conventions

- JSON shared with TS/Tauri uses `#[serde(rename_all = "camelCase")]` (about 260 types, e.g.
  `src/scm.rs::DiffIndex`). Enums of fixed tags use `rename_all = "lowercase"`
  (`src/app_update/mod.rs::PackageKind`).
- Tolerant reading: `#[serde(default)]` on structs/fields and `skip_serializing_if` for optional
  output are the norm. `deny_unknown_fields` is used only for strictly validated input
  (`src/visual/model.rs`); do not add it to stored data, it breaks reading files from newer apps.
- Renaming a stored field: keep reading the old name with `#[serde(alias = "...")]`, as
  `src/agent.rs::TurnMetrics` does for its camelCase spellings.
- Numbers: `serde_json` is built with `float_roundtrip` ("Stored floats must reload exactly, or
  saved revisions stop matching", `Cargo.toml`). JS-compatible number output uses
  `mono::workflows::ordered::write_number` (whole values without a fraction).
- Key order: the app build enables serde_json `preserve_order`, `cargo test -p monocode-core`
  does not. Where order is visible or byte-compared, use the order-preserving
  `mono::workflows::ordered::Json`/`OrderedMap`, not `serde_json::Map`
  (`src/mono/workflows/ordered.rs` header explains why).

## Byte-for-byte fixtures

- `src/mono/workflows/fixtures/**` were generated by running the real TS code
  (`workflowYaml.ts`, `workflowStore.ts`) and are compared byte for byte
  (`yaml_tests.rs::FIXTURES`, `store_tests.rs::a_file_written_by_the_ts_store_round_trips_byte_identical`).
- `src/app_update/fixtures/package.txt` is signature-checked, so its bytes must be identical on
  every platform.
- Both are pinned `text eol=lf` in the repo-root `.gitattributes`. A new byte-compared fixture
  needs its own `eol=lf` entry there, or a Windows CRLF checkout fails the test. Regenerate such
  fixtures from the source app; never hand-edit them to make a test pass.

## Identifiers that never change

From `CLAUDE.md` "Never rename these" plus what this crate encodes:

- `src/brand.rs` header: "file names, env vars and protocol strings keep their `monocode`
  spelling". Product copy and bundle ids (`PRODUCT`, `DESKTOP_APP_ID = "com.monoptah.desktop"`,
  `NATIVE_APP_ID = "com.monoptah.native"`) live there; the updater matches packages on
  `NATIVE_APP_ID`.
- The database file name `monocode.db` and the sidecar names `.gpui.sqlite3` / `.gpui.json`.
- `monocode.*` localStorage keys, read from the desktop WebView by `src/desktop_preferences.rs`
  ("without modifying WebKit storage or its schema"; Windows WebView2 LevelDB in
  `desktop_preferences/webview.rs`). Native reads them; it never writes them.
- `monocode-host` and its `~/.monocode-host/` data dir (`src/remote/`), and env vars such as
  `MONOCODE_HARNESS_PARENT` (`src/process.rs::isolate`).
- Crate names `monocode-core`/`monocode-gpui`: scripts and release artifacts match on them.
- Sentinel error strings (see [error-handling.md](./error-handling.md)).

If a branding hit from `scripts/check-fork-branding.sh` is one of these identifiers, add it to
that script's ALLOW list with a reason instead of renaming it.
