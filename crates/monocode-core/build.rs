//! Capture the source that built the probe, including dirty/untracked native code.
use sha2::{Digest, Sha256};
use std::{path::Path, process::Command};

fn output(root: &Path, program: &str, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new(program)
        .current_dir(root)
        .args(args)
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn source(path: &str) -> bool {
    path.ends_with(".rs")
        || path.ends_with(".sql")
        || path.ends_with("Cargo.toml")
        || path.ends_with("Cargo.lock")
        || path.ends_with(".cargo/config")
        || path.ends_with(".cargo/config.toml")
}

fn main() {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").unwrap();
    let root = Path::new(&manifest).parent().unwrap().parent().unwrap();
    let scope = [
        "crates/monocode-core",
        "crates/monocode-gpui",
        "crates/mobile-link",
        "vendor",
        "Cargo.toml",
        "Cargo.lock",
        ".cargo",
    ];
    // Identity still hashes the full declared scope (`scope`) for accuracy,
    // but only monocode-core's own files and the native workspace manifests
    // retrigger this script, so ordinary gpui edits and git operations do not
    // recompile monocode-core. Before building the probe for a measurement,
    // run `touch crates/monocode-core/build.rs` to force an identity refresh.
    for path in [
        "crates/monocode-core/src",
        "crates/monocode-core/examples",
        "crates/monocode-core/build.rs",
        "crates/monocode-core/Cargo.toml",
        "crates/monocode-gpui/Cargo.toml",
        "crates/monocode-gpui/Cargo.lock",
        "crates/monocode-gpui/build.rs",
        "crates/monocode-gpui/.cargo",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    println!("cargo:rerun-if-env-changed=RUSTC");
    let sha = output(root, "git", &["rev-parse", "HEAD"])
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|text| text.trim().to_owned());
    let mut args = vec![
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
        "--",
    ];
    args.extend(scope);
    let files = output(root, "git", &args).and_then(|bytes| String::from_utf8(bytes).ok());
    let mut tree = Sha256::new();
    let mut diff = Sha256::new();
    let mut complete = sha.is_some() && files.is_some();
    if let Some(files) = files {
        let mut files = files
            .split('\0')
            .filter(|path| !path.is_empty() && source(path))
            .collect::<Vec<_>>();
        files.sort_unstable();
        files.dedup();
        let mut args = vec!["diff", "HEAD", "--binary", "--"];
        args.extend(files.iter().copied());
        if let Some(patch) = output(root, "git", &args) {
            diff.update(patch);
        } else {
            complete = false;
        }
        for path in files {
            match std::fs::read(root.join(path)) {
                Ok(bytes) => {
                    tree.update((path.len() as u64).to_le_bytes());
                    tree.update(path.as_bytes());
                    tree.update((bytes.len() as u64).to_le_bytes());
                    tree.update(&bytes);
                    // Tracked bytes are already represented by the diff, but include
                    // every current source byte here too so untracked code cannot hide.
                    diff.update((path.len() as u64).to_le_bytes());
                    diff.update(path.as_bytes());
                    diff.update((bytes.len() as u64).to_le_bytes());
                    diff.update(bytes);
                }
                Err(_) => complete = false,
            }
        }
    }
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let toolchain = output(root, &rustc, &["-vV"]).and_then(|bytes| String::from_utf8(bytes).ok());
    let lock = std::fs::read(root.join("crates/monocode-gpui/Cargo.lock"))
        .ok()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
    let out = std::env::var_os("OUT_DIR").unwrap();
    let profile = Path::new(&out)
        .ancestors()
        .nth(3)
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned());
    complete &= toolchain.is_some() && lock.is_some() && profile.is_some();
    let identity = serde_json::json!({"complete":complete, "source_sha":sha,
        "diff_sha256":format!("{:x}",diff.finalize()), "source_tree_sha256":format!("{:x}",tree.finalize()),
        "toolchain":toolchain, "cargo_lock_sha256":lock, "profile":profile});
    std::fs::write(
        Path::new(&out).join("persistence_probe_build.json"),
        identity.to_string(),
    )
    .unwrap();
}
