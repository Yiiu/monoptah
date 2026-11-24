//! Bounded CLI invocations for Settings management. Call only from workers.
//! Also holds the per-agent CLI path overrides and extra launch configuration that every
//! launch reads; both are replaced whenever the settings change.
pub use crate::launch_args::Launch;
use crate::provider::Kind;
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{OnceLock, RwLock},
    time::{Duration, Instant},
};
fn paths() -> &'static RwLock<HashMap<String, String>> {
    static PATHS: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    PATHS.get_or_init(Default::default)
}
/// Replace the configured CLI paths. Called at startup and after every settings change,
/// so a new path applies to the next launch without a restart.
pub fn initialize(overrides: &HashMap<String, String>) {
    let mut paths = paths().write().unwrap_or_else(|e| e.into_inner());
    if *paths != *overrides {
        *paths = overrides.clone();
    }
}
fn launches() -> &'static RwLock<HashMap<String, Launch>> {
    static LAUNCHES: OnceLock<RwLock<HashMap<String, Launch>>> = OnceLock::new();
    LAUNCHES.get_or_init(Default::default)
}
/// Replace every agent's extra launch arguments and environment, keyed by `Kind::id()`.
/// Called with the settings at startup and after every settings change.
pub fn set_launches(next: HashMap<String, Launch>) {
    let mut launches = launches().write().unwrap_or_else(|e| e.into_inner());
    if *launches != next {
        *launches = next;
    }
}
/// The extra arguments and environment the user set for this agent; empty when none.
pub fn launch(kind: Kind) -> Launch {
    launches()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(kind.id())
        .cloned()
        .unwrap_or_default()
}
/// The agent's configured command as the program to start. A bare name is looked up on
/// the GUI search path, as a terminal would; a command that does not resolve is returned
/// as typed, so the launch fails with its own error instead of silently using another CLI.
pub fn configured(kind: Kind) -> Option<PathBuf> {
    let command = paths()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(kind.id())
        .filter(|p| !p.trim().is_empty())
        .cloned()?;
    Some(resolve_command(&command).unwrap_or_else(|_| PathBuf::from(command.trim())))
}
/// Resolve what the user entered as an agent's command: a bare name (`claude-nightly`,
/// `claude.cmd`) found on the GUI search path, with the Windows executable extensions
/// tried for a name without one, or an absolute path (`~` allowed).
pub fn resolve_command(command: &str) -> Result<PathBuf, String> {
    resolve_command_in(command, &crate::scm::shell::gui_search_path())
}
/// `resolve_command` against a given search path, so tests need not change `PATH`.
pub(crate) fn resolve_command_in(command: &str, search_path: &str) -> Result<PathBuf, String> {
    let command = command.trim();
    let typed = Path::new(command);
    let bare = !command.starts_with('~')
        && !typed.is_absolute()
        && typed.components().count() == 1
        && !command.contains(['/', '\\']);
    if bare {
        return crate::scm::shell::which_in_path(search_path, command).ok_or_else(|| {
            format!(
                "\"{command}\" was not found on your PATH. Enter a command on your PATH or the full path to the CLI."
            )
        });
    }
    let path = crate::scm::shell::expand_home(typed);
    if !path.is_absolute() {
        return Err("Enter a command name or an absolute path to the CLI executable.".into());
    }
    crate::scm::shell::existing_binary(path.clone())
        .or_else(|| path.is_file().then(|| path.clone()))
        .ok_or_else(|| format!("{} does not exist or is not a file.", path.display()))
}
pub fn output(
    binary: &Path,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
) -> Result<String, String> {
    let mut command = Command::new(binary);
    crate::scm::shell::apply_gui_env(&mut command);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::process::isolate(&mut command);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let _registration = crate::process::register(child.id());
    let read = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut result = Vec::new();
            let mut chunk = [0; 4096];
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                result.extend_from_slice(&chunk[..n]);
                if result.len() > 65536 {
                    result.drain(..result.len() - 65536);
                }
            }
            String::from_utf8_lossy(&result).trim().to_owned()
        })
    };
    let stdout = read(Box::new(child.stdout.take().unwrap()));
    let stderr = read(Box::new(child.stderr.take().unwrap()));
    let deadline = Instant::now() + timeout;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(e) => break Err(e.to_string()),
            Ok(None) if Instant::now() >= deadline => break Err("CLI command timed out".into()),
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    crate::process::terminate(&mut child);
    let out = stdout.join().unwrap_or_default();
    let err = stderr.join().unwrap_or_default();
    if result?.success() {
        Ok(out)
    } else {
        Err(format!("{err} {out}").trim().to_owned())
    }
}
#[derive(Clone, Debug)]
pub struct Inspection {
    pub path: PathBuf,
    pub version: String,
}
pub fn inspect(kind: Kind, path: Option<&str>) -> Result<Inspection, String> {
    inspect_in(kind, path, &crate::scm::shell::gui_search_path())
}
/// `inspect` against a given search path. A command the user entered may be a wrapper or a
/// differently named build, so only the auto-detected CLI must print the expected version
/// format; a version that can be read is still checked against the minimum.
pub(crate) fn inspect_in(
    kind: Kind,
    path: Option<&str>,
    search_path: &str,
) -> Result<Inspection, String> {
    let entered = path.filter(|s| !s.trim().is_empty());
    let path = match entered {
        Some(command) => resolve_command_in(command, search_path)?,
        None => crate::provider::resolve_auto(kind).ok_or("CLI could not be resolved")?,
    };
    if !path.is_file() {
        return Err(format!("{} does not exist.", path.display()));
    }
    let cwd = crate::scm::shell::dirs_home()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    let version =
        output(&path, &["--version".into()], &cwd, Duration::from_secs(10)).map_err(|error| {
            format!(
                "`{} --version` failed: {}",
                path.display(),
                if error.is_empty() {
                    "it exited with an error and printed nothing"
                } else {
                    error.as_str()
                }
            )
        })?;
    if kind == Kind::Codex
        && entered.is_none()
        && !regex::Regex::new(r"^codex-cli\s+\d+\.\d+\.\d+")
            .unwrap()
            .is_match(&version)
    {
        return Err(format!(
            "Codex CLI returned an invalid version: {}",
            version.lines().next().unwrap_or_default()
        ));
    }
    if kind == Kind::OpenCode {
        let v = regex::Regex::new(r"(?:^|\s)v?(\d+)\.(\d+)\.(\d+)").unwrap();
        let Some(c) = v.captures(&version) else {
            if entered.is_some() {
                return Ok(Inspection { path, version });
            }
            return Err(format!(
                "OpenCode CLI returned an invalid version: {}",
                version.lines().next().unwrap_or_default()
            ));
        };
        let major = c[1].parse::<u32>().unwrap_or(0);
        let minor = c[2].parse::<u32>().unwrap_or(0);
        let patch = c[3].parse::<u32>().unwrap_or(0);
        if (major, minor, patch) < (1, 14, 19) {
            return Err(format!(
                "OpenCode v{major}.{minor}.{patch} is too old. Upgrade to v1.14.19 or newer."
            ));
        }
    }
    Ok(Inspection { path, version })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn validates_executable_and_provider_version_before_saving() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLI with spaces");
        std::fs::write(&path, "#!/bin/sh\nprintf 'codex-cli 1.2.3\\n'\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            inspect(Kind::Codex, path.to_str()).unwrap().version,
            "codex-cli 1.2.3"
        );
        assert!(inspect(Kind::Codex, Some("relative/path"))
            .unwrap_err()
            .contains("absolute"));
        std::fs::write(&path, "#!/bin/sh\nprintf '1.10.3\\n'\n").unwrap();
        assert!(inspect(Kind::OpenCode, path.to_str())
            .unwrap_err()
            .contains("1.14.19"));
        // A command the user entered may be a wrapper: its version text is not policed.
        assert_eq!(
            inspect(Kind::Codex, path.to_str()).unwrap().version,
            "1.10.3"
        );
        // The real reason a CLI could not be checked is reported.
        std::fs::write(&path, "#!/bin/sh\necho 'no such option' >&2\nexit 2\n").unwrap();
        let error = inspect(Kind::Claude, path.to_str()).unwrap_err();
        assert!(error.contains("--version"), "{error}");
        assert!(error.contains("no such option"), "{error}");
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;

    /// An executable named `name` in `dir`: a script on Unix, a `.cmd` shim on Windows.
    fn executable(dir: &Path, name: &str) -> PathBuf {
        #[cfg(windows)]
        let path = dir.join(format!("{name}.cmd"));
        #[cfg(not(windows))]
        let path = dir.join(name);
        std::fs::write(&path, "echo 1.0.0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn a_bare_command_name_resolves_on_the_search_path() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let shim = executable(second.path(), "claude-nightly");
        let search = std::env::join_paths([first.path(), second.path()])
            .unwrap()
            .to_string_lossy()
            .into_owned();
        // A name other than the agent's own, without an extension.
        assert_eq!(resolve_command_in("claude-nightly", &search).unwrap(), shim);
        assert_eq!(
            resolve_command_in("  claude-nightly ", &search).unwrap(),
            shim
        );
        // The file name as it is on disk (with its extension on Windows).
        let file_name = shim.file_name().unwrap().to_str().unwrap();
        assert_eq!(resolve_command_in(file_name, &search).unwrap(), shim);
        // A full path needs no search path.
        assert_eq!(
            resolve_command_in(shim.to_str().unwrap(), "").unwrap(),
            shim
        );
    }

    #[test]
    fn a_command_that_does_not_resolve_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let search = dir.path().to_string_lossy().into_owned();
        let missing = resolve_command_in("claude-missing", &search).unwrap_err();
        assert!(missing.contains("claude-missing"), "{missing}");
        assert!(missing.contains("PATH"), "{missing}");
        assert!(resolve_command_in("relative/path", &search)
            .unwrap_err()
            .contains("absolute"));
        let gone = dir.path().join("gone");
        let error = resolve_command_in(gone.to_str().unwrap(), &search).unwrap_err();
        assert!(error.contains("does not exist"), "{error}");
        // `inspect` reports the same reason instead of a generic one.
        assert_eq!(
            inspect_in(Kind::Claude, Some("claude-missing"), &search).unwrap_err(),
            missing
        );
    }
}

#[cfg(test)]
mod reload_tests {
    use super::*;

    #[test]
    fn paths_and_launches_are_replaced_on_every_call() {
        // Fx is never launched by the core tests, so this does not leak into others.
        let first = HashMap::from([("fx".to_string(), "/first/fx".to_string())]);
        initialize(&first);
        assert_eq!(configured(Kind::Fx), Some(PathBuf::from("/first/fx")));
        let second = HashMap::from([("fx".to_string(), "/second/fx".to_string())]);
        initialize(&second);
        assert_eq!(configured(Kind::Fx), Some(PathBuf::from("/second/fx")));
        initialize(&HashMap::new());
        assert_eq!(configured(Kind::Fx), None);

        let launch = Launch {
            args: vec!["--verbose".into()],
            env: vec![("HTTP_PROXY".into(), "http://127.0.0.1:9".into())],
        };
        set_launches(HashMap::from([("fx".to_string(), launch.clone())]));
        assert_eq!(super::launch(Kind::Fx), launch);
        set_launches(HashMap::new());
        assert!(super::launch(Kind::Fx).is_empty());
    }
}
