# Error Handling and Logging

## The default: `Result<T, String>`

Commands and nearly all internal functions return `Result<T, String>`. There is no crate
error enum and no `anyhow`; `thiserror` is a workspace dependency but `src-tauri/src` does
not use it. The `String` reaches the UI as the rejection value of `invoke`, so it is
written for a person to read.

Conversion patterns actually used:

```rust
// Plain: library error text is good enough.
Connection::open(&path).map_err(|e| e.to_string())?;          // session_store.rs
// With context: say what failed and on which path/program.
std::fs::copy(&source, &temp).map_err(|e| format!("{}: {e}", temp.display()))?;
spawn_managed(&mut cmd).map_err(|e| format!("Failed to start {command}: {e}"))?; // harness.rs
// Replace an opaque error with a message the user can act on.
serde_json::from_str(&raw).map_err(|_| "ClickUp settings are invalid".to_string())?; // clickup.rs
// Missing state: ok_or_else with a sentence.
host.get(&id).ok_or_else(|| "Terminal is not running".to_string())?;  // pty.rs
```

Rules drawn from those call sites:

- Messages are short English sentences a user can read, usually without a trailing
  period (`"Working directory does not exist: {}"`, `"Harness process is not running"`);
  a second sentence may tell the user what to do (`control.rs`: `"Control request timed
  out. Retry with the same request ID."`).
  Guard failures that indicate a webview bug may lead with the command name
  (`"pty_spawn: not a resolved harness CLI"`).
- Never put tokens, passwords, or credential file contents into an error string; these
  strings are shown in the UI and may be forwarded to the control CLI.
- User-visible text says "Monoptah", not "MonoCode"
  (`control.rs`: `"Monoptah executor is unavailable"`). `scripts/check-fork-branding.sh`
  flags quoted "MonoCode" strings; see CLAUDE.md "After every merge from upstream".
- Validate at the top of the command and return early
  (`session_upsert`: `validate_id(...)?;` then `"cwd is required"`).

## Structured errors (the exceptions)

Use a typed error only when the caller must branch on more than the message:

- `mail_send::SendError { kind, message, retry_safe }` — `Serialize`, `camelCase`, and the
  only command error that is not a `String` (`mail_outbox_send` returns
  `Result<_, SendError>`). It deliberately has **no `From<String>`**: "a bare `?` on a
  string error would pick a retry-safety for the caller". The TS side reads
  `SendError.retrySafe` (`src/features/mail/model/mail.ts`, `mailOutbox.ts`).
- Module-internal enums that are mapped to strings before leaving the module:
  `contributors::gh::GhError { NotInstalled, Failed(String) }`,
  `telegram::api::TgError`, `aws_s3::S3ErrorKind`, `aws_ses::SesErrorKind`.
- `crates/mobile-link`: `LinkResult<T> = Result<T, String>`, plus
  `Failure { error, code: Option<String> }` with `Failure::coded(code, msg)` when a client
  needs a stable code; a `Failure` without a code is written exactly as a plain text error
  "so the QUIC wire is unchanged" (`crates/mobile-link/src/lib.rs`).

## Locks

Two patterns are used for `std::sync::Mutex`:

- Process hosts recover from poisoning so a panic in one thread never bricks
  terminals or agents: `.lock().unwrap_or_else(|e| e.into_inner())`
  (`pty::PtyHost`, `harness::harness_write`).
- Stores convert poisoning into an error: `.lock().map_err(|_| "Session store is
  locked".to_string())` (`SessionStore::lock_conn`, `with_read_conn`).

Do not hold a store lock across git, network, or process calls:
`SessionStore::with_read_conn` says "Never call git or the network inside `f`", and the
test helper `conn_is_free` exists to prove long operations release the lock.

## Startup errors

In `lib.rs` `.setup(...)`, an `init` that returns `Err` aborts the launch (`?`):
`session_store::init`, `control::init`, `tui_hook::init`, `mobile_access::init`,
`checkpoint::init`, `menu::install`. Features that are optional at startup have `init`
return `()` and handle failure inside (`reminders::init`, `telegram::init`,
`mail::init`). Pick deliberately: a new optional feature must not be able to stop the app
from opening. `.expect(...)` is limited to unrecoverable boot steps
(`windows::initialize().expect("Failed to initialize Windows process safety")`,
`.build(context).expect("error while building Monoptah")`).

## Ignoring results

`let _ = ...` is the convention for best-effort side effects whose failure has no useful
handler: emits (`let _ = app.emit(...)`), window cosmetics
(`let _ = window.set_shadow(true)`), and kills during shutdown. Do not use it on writes to
user data or on anything a command reports back.

## Logging

There is no logging framework (no `log`, `tracing`, or plugin-log). Diagnostics that the
user cannot act on go to stderr with `eprintln!`, prefixed with `monocode:` and a short
area, a convention kept from upstream:

```rust
eprintln!("monocode: quick composer: {err}");          // quick_composer.rs
eprintln!("monocode: macos dev bundle: {err}");        // macos.rs
```

A few modules use a descriptive prefix instead (`provider_session_cleanup.rs`:
`"Provider session cleanup: {problem}"`). `println!` is used only where stdout is the
interface: the CLI entry points (`control_cli.rs`, `ssh_askpass.rs`) and a test child
process in `windows.rs` tests. Anything the user needs to know is returned as
an `Err` or emitted as an event, not logged.
