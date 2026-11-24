# Error Handling

> How `monocode-core` reports failures, and how they reach the user.

---

## The norm: `Result<T, String>` with user-facing text

The crate uses neither `anyhow` nor `thiserror` (neither is in `crates/monocode-core/Cargo.toml`).
Almost every fallible function returns `Result<T, String>` (about 1,900 occurrences in `src/`).
The `String` is **the message the user will read**: GPUI stores it in view state and renders it
(`crates/monocode-gpui/src/accounts.rs`: `Err(error) => this.error = Some(error)`). So:

- Write the error as a complete English sentence a user can act on, naming the place to fix it.
  Examples: `src/docker_accounts.rs` `DockerAuthError` ("token revoked or expired. Replace it in
  Settings > Cloud."), `src/accounts/token.rs` `CREDENTIAL_UNREADABLE` ("Replace the token or API
  key in Accounts."), `src/git.rs` `git()` ("Could not run Git: {e}").
- Product name in copy is "Monoptah" (`src/brand.rs::PRODUCT`), never "MonoCode";
  `scripts/check-fork-branding.sh` scans `crates/monocode-core/src/`.
- Keep app copy in English (user-authored text stays in its own language).
- Library errors are converted at the boundary with `map_err(|e| e.to_string())`. Larger modules
  wrap that once: `src/store/shared.rs` has `fn sql<T>(r: rusqlite::Result<T>) -> Result<T, String>`
  and `fn parse<T>(raw) -> Result<T, String>`. Add context when the raw text alone would confuse:
  `src/mono/files.rs::write_atomic` prefixes the path (`format!("{}: {e}", path.display())`),
  `src/store.rs` uses `format!("Cannot read provider accounts: {e}")`.
- Internal helpers that only feed another layer may keep the library error and convert later
  (`src/store/notes.rs::ensure_notes_table` returns `rusqlite::Result<()>`).

## Typed errors when the caller must branch

Introduce a struct/enum only when a caller needs more than the message. Every typed error
implements `Display` with user text (and usually `std::error::Error`), so `.to_string()` still
yields something showable:

| Type | File | Why it is typed |
|------|------|-----------------|
| `CreateError { id, message, already_exists }` | `src/sandboxes.rs` | A failed create may still have made a sandbox that bills; caller must adopt or clean up `id` |
| `RewindError { message, provider_may_have_changed }` | `src/provider/rewind.rs` | Caller must know whether provider state moved |
| `DockerAuthError` (enum) | `src/docker_accounts.rs` | Distinct remedies per cause; also flattened to `String` by `sandboxes::run_as_account` |
| `ConversionError`, `FlushError` | `src/store/native_chunks/` | Storage conversion stages |
| `NoiseError`, `SendError`, `PushError` | `src/mobile/web_link/` | Transport outcomes |

Typed errors provide `From<String>` / `From<&str>` so `?` keeps working inside String-based code
(`impl From<String> for CreateError` derives `already_exists` from the text).

## Sentinel strings are contracts

Some error strings are compared by value. Changing their text silently breaks the caller:

- `crate::mono::files::CONFLICT`: matched with `Err(e) if e == files::CONFLICT` in
  `src/mono/habits.rs`, `src/mono/heartbeat.rs` and `crates/monocode-gpui/src/monos/mobile.rs`.
- `checkpoint::gate::CLOUD_OWNED` (`src/checkpoint/gate.rs`), returned and compared by GPUI
  (`crates/monocode-gpui/src/conversation/cloud_location.rs`).
- `UNSUPPORTED_STORAGE_MODE` (`"UnsupportedStorageMode"`, `src/store/native_chunks/mod.rs`) is a
  prefix on refusal messages so the kind is recognisable.
- CLI-output classifiers are deliberately narrow: `sandboxes::is_not_found_error` matches only
  `sandbox '<ref>' not found`, because "a false positive makes the app forget a sandbox that is
  still billing". Do not broaden such matchers to generic "not found".

When you add a sentinel, make it a `pub const` and compare against the constant, never a literal.

## Secrets never appear in error text

- Output of external CLIs passes through `sandboxes::redact` before it becomes an error
  (`sandboxes::failure`: "stderr, else stdout, redacted"). `redact` masks URL userinfo, bearer
  tokens, JSON `token`/`password`, `KEY=value` and known prefixes (`ghp_`, `sk-ant-`,
  `dckr_pat_`, ...). Reuse it for any CLI that may echo credentials.
- Credential code maps every failure to a fixed message instead of the underlying error
  (`src/accounts/token.rs`: `.map_err(|_| SAVE_FAILED)`), so key material and paths cannot leak.
- Log lines about pushes are "Metadata only: never a token, URL, title or message text"
  (`src/mobile/notify.rs::log_line`).

## Panics

- For anything driven by input, files, the network or another process, return a `String` error
  rather than `panic!`/`expect`: a panic on a background worker loses the user's operation with
  no message. `expect`/`unwrap` are used in tests and for true invariants only.
- Mutex poisoning: the established pattern is `lock().unwrap_or_else(|e| e.into_inner())` (or
  `PoisonError::into_inner`) so one panicked worker does not take down every later caller.
  Plain `lock().unwrap()` also exists, mostly in tests and fakes (`src/sandboxes/test_support.rs`).
- Best-effort side steps ignore their result explicitly with `let _ =` and a comment saying why
  (`src/process.rs::configure_child`: "a refused hint must not prevent a provider from launching";
  `sandboxes::run_piped`: "A closed pipe means sbx exited early; its status reports why").

## Logging

There is no logging crate (`log`/`tracing` are unused). Diagnostics go to stderr with
`eprintln!` (about 40 call sites), using a short prefix that names the subsystem:

```rust
eprintln!("orphan guard: watchdog did not start: {error}");        // src/process/guard.rs
eprintln!("[web-push] category={category} suppressed: {reason}");  // src/mobile/notify.rs
```

- Log only when the error has nowhere better to go (a background tick, a cleanup step). If a
  caller can show it, return it instead. `src/mono/workflows/tick.rs::warn` is the pattern for
  background work.
- Release builds have no console on Windows or when launched from Finder. Anything the user must
  be able to see later goes to an in-app record as well (`src/mobile/notify.rs::report` writes
  stderr *and* the audit log shown in Settings).
- Never log tokens, URLs with credentials, prompts or message text.
