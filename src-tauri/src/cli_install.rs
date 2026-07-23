//! "Install CLI" action: symlink the bundled CLI sidecar onto the user's
//! PATH, VS Code-style. A symlink (never a copy) keeps the CLI the same
//! build as the app, so app updates propagate with no reinstall.
//!
//! The link is named `prologue` but the bundled binary is `prologue-cli`:
//! the two names must differ in more than case, because on a
//! case-insensitive filesystem `prologue` and the main app binary
//! `Prologue` are the same file — the bundler would collapse them and the
//! "CLI" would launch the GUI.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// Name of the symlink placed on the user's PATH.
pub const CLI_NAME: &str = "prologue";
/// Name of the sidecar inside Contents/MacOS (externalBin minus the triple).
pub const BUNDLED_CLI_NAME: &str = "prologue-cli";

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InstallStatus {
    /// Symlink created fresh.
    Installed,
    /// A symlink existed but pointed elsewhere (e.g. an old app location);
    /// it was replaced.
    ReplacedStale,
    /// Correct symlink already in place; nothing changed.
    AlreadyInstalled,
    /// App is not running from /Applications (and no force override):
    /// a symlink into a translocated or build-dir path would dangle.
    NotApplicable,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub status: InstallStatus,
    pub symlink_path: Option<String>,
    pub target: Option<String>,
    /// Set when the symlink landed in a directory that is not on PATH.
    pub path_hint: Option<String>,
    pub message: String,
}

/// Everything the install decision depends on, injectable for tests.
pub struct InstallEnv {
    /// The running app binary.
    pub exe_path: PathBuf,
    /// Directory holding the running app binary (and the bundled CLI).
    pub exe_dir: PathBuf,
    pub in_applications: bool,
    pub usr_local_bin: PathBuf,
    pub home_local_bin: PathBuf,
    /// The PATH variable, for the fallback hint.
    pub path_var: String,
}

impl InstallEnv {
    pub fn detect() -> Result<Self, String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("cannot locate the running app binary: {e}"))?;
        let exe_dir = exe
            .parent()
            .ok_or("app binary has no parent directory")?
            .to_path_buf();
        // A quarantined app runs translocated from /private/var/...; only a
        // real /Applications path yields a symlink that survives relaunch.
        let in_applications = exe.starts_with("/Applications/");
        let home = std::env::var("HOME").map_err(|_| "HOME is not set")?;
        Ok(Self {
            exe_path: exe,
            exe_dir,
            in_applications,
            usr_local_bin: PathBuf::from("/usr/local/bin"),
            home_local_bin: Path::new(&home).join(".local/bin"),
            path_var: std::env::var("PATH").unwrap_or_default(),
        })
    }
}

fn dir_is_writable(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let probe = dir.join(format!(".{CLI_NAME}-install-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn on_path(dir: &Path, path_var: &str) -> bool {
    std::env::split_paths(path_var).any(|p| p == dir)
}

/// True when both paths resolve to the same inode (so a case-insensitive
/// name collision is caught even though the paths differ as strings).
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(ma), Ok(mb)) => ma.dev() == mb.dev() && ma.ino() == mb.ino(),
        _ => false,
    }
}

pub fn install(env: &InstallEnv, force: bool) -> Result<InstallReport, String> {
    if !env.in_applications && !force {
        return Ok(InstallReport {
            status: InstallStatus::NotApplicable,
            symlink_path: None,
            target: None,
            path_hint: None,
            message: format!(
                "Move the app to /Applications first — a link to {} would break \
                 on relaunch.",
                env.exe_dir.display()
            ),
        });
    }

    let target = env.exe_dir.join(BUNDLED_CLI_NAME);
    if !target.is_file() {
        return Err(format!(
            "bundled {BUNDLED_CLI_NAME} not found at {}",
            target.display()
        ));
    }
    // On a case-insensitive filesystem a sidecar whose name matches the app
    // binary in all but case collapses into the app binary at bundling time.
    // Fail loudly rather than install a symlink that launches the GUI.
    if same_file(&target, &env.exe_path) {
        return Err(format!(
            "bundled {BUNDLED_CLI_NAME} at {} is the app binary itself — \
             refusing to install a CLI link that would launch the app",
            target.display()
        ));
    }

    let (bin_dir, fell_back) = if dir_is_writable(&env.usr_local_bin) {
        (env.usr_local_bin.clone(), false)
    } else {
        std::fs::create_dir_all(&env.home_local_bin)
            .map_err(|e| format!("cannot create {}: {e}", env.home_local_bin.display()))?;
        (env.home_local_bin.clone(), true)
    };
    let link = bin_dir.join(CLI_NAME);

    let mut status = InstallStatus::Installed;
    match std::fs::symlink_metadata(&link) {
        Ok(meta) if meta.file_type().is_symlink() => {
            if std::fs::read_link(&link).ok().as_deref() == Some(&target) {
                status = InstallStatus::AlreadyInstalled;
            } else {
                std::fs::remove_file(&link)
                    .map_err(|e| format!("cannot replace {}: {e}", link.display()))?;
                status = InstallStatus::ReplacedStale;
            }
        }
        Ok(_) => {
            return Err(format!(
                "{} exists and is not a symlink — not overwriting it",
                link.display()
            ));
        }
        Err(_) => {}
    }
    if status != InstallStatus::AlreadyInstalled {
        std::os::unix::fs::symlink(&target, &link)
            .map_err(|e| format!("cannot create {}: {e}", link.display()))?;
    }

    let path_hint = (!on_path(&bin_dir, &env.path_var)).then(|| {
        format!(
            "add it to your PATH: export PATH=\"{}:$PATH\"",
            bin_dir.display()
        )
    });
    let message = match status {
        InstallStatus::AlreadyInstalled => format!("{CLI_NAME} is already installed."),
        _ if fell_back => format!(
            "Installed {CLI_NAME} to {} (/usr/local/bin is not writable).{}",
            link.display(),
            path_hint
                .as_deref()
                .map(|h| format!(" To use it, {h}"))
                .unwrap_or_default()
        ),
        _ => format!("Installed {CLI_NAME} to {}.", link.display()),
    };

    Ok(InstallReport {
        status,
        symlink_path: Some(link.display().to_string()),
        target: Some(target.display().to_string()),
        path_hint,
        message,
    })
}

/// `force` skips the /Applications guard — a dev/testing override so the
/// action can be exercised against a debug build (documented in README).
#[tauri::command]
pub fn install_cli(force: Option<bool>) -> Result<InstallReport, String> {
    install(&InstallEnv::detect()?, force.unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        env: InstallEnv,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "cli-install-test-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let exe_dir = root.join("App.app/Contents/MacOS");
            std::fs::create_dir_all(&exe_dir).unwrap();
            let exe_path = exe_dir.join("AppBinary");
            std::fs::write(&exe_path, b"#!/bin/sh\n# app\n").unwrap();
            std::fs::write(exe_dir.join(BUNDLED_CLI_NAME), b"#!/bin/sh\n").unwrap();
            let usr_local_bin = root.join("usr-local-bin");
            std::fs::create_dir_all(&usr_local_bin).unwrap();
            let env = InstallEnv {
                exe_path,
                exe_dir,
                in_applications: true,
                usr_local_bin,
                home_local_bin: root.join("home/.local/bin"),
                path_var: String::new(),
            };
            Self { root, env }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Restore permissions so cleanup can remove read-only dirs.
            let mut perms = std::fs::metadata(&self.env.usr_local_bin)
                .unwrap()
                .permissions();
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&self.env.usr_local_bin, perms);
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn refuses_outside_applications_without_force() {
        let mut f = Fixture::new("guard");
        f.env.in_applications = false;
        let report = install(&f.env, false).unwrap();
        assert_eq!(report.status, InstallStatus::NotApplicable);
        assert!(!f.env.usr_local_bin.join(CLI_NAME).exists());
    }

    #[test]
    fn force_overrides_the_applications_guard() {
        let mut f = Fixture::new("force");
        f.env.in_applications = false;
        let report = install(&f.env, true).unwrap();
        assert_eq!(report.status, InstallStatus::Installed);
    }

    #[test]
    fn installs_symlink_into_writable_usr_local_bin() {
        let f = Fixture::new("install");
        let report = install(&f.env, false).unwrap();
        assert_eq!(report.status, InstallStatus::Installed);
        let link = f.env.usr_local_bin.join(CLI_NAME);
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            f.env.exe_dir.join(BUNDLED_CLI_NAME)
        );
        // The fixture's PATH is empty, so even the primary dir gets a hint.
        assert!(report.path_hint.is_some());
    }

    #[test]
    fn second_install_is_a_noop() {
        let f = Fixture::new("idempotent");
        install(&f.env, false).unwrap();
        let report = install(&f.env, false).unwrap();
        assert_eq!(report.status, InstallStatus::AlreadyInstalled);
    }

    #[test]
    fn stale_symlink_is_replaced() {
        let f = Fixture::new("stale");
        let link = f.env.usr_local_bin.join(CLI_NAME);
        std::os::unix::fs::symlink(f.root.join("old-app-location"), &link).unwrap();
        let report = install(&f.env, false).unwrap();
        assert_eq!(report.status, InstallStatus::ReplacedStale);
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            f.env.exe_dir.join(BUNDLED_CLI_NAME)
        );
    }

    #[test]
    fn regular_file_at_link_path_is_never_overwritten() {
        let f = Fixture::new("realfile");
        let link = f.env.usr_local_bin.join(CLI_NAME);
        std::fs::write(&link, b"user's own script").unwrap();
        let err = install(&f.env, false).unwrap_err();
        assert!(err.contains("not a symlink"));
        assert_eq!(std::fs::read(&link).unwrap(), b"user's own script");
    }

    #[test]
    fn falls_back_to_home_local_bin_when_usr_local_bin_unwritable() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new("fallback");
        std::fs::set_permissions(
            &f.env.usr_local_bin,
            std::fs::Permissions::from_mode(0o555),
        )
        .unwrap();
        let report = install(&f.env, false).unwrap();
        assert_eq!(report.status, InstallStatus::Installed);
        let link = f.env.home_local_bin.join(CLI_NAME);
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert!(report.path_hint.is_some());
        assert!(report.message.contains("not writable"));
    }

    #[test]
    fn no_path_hint_when_bin_dir_is_on_path() {
        let mut f = Fixture::new("onpath");
        f.env.path_var = std::env::join_paths([
            Path::new("/somewhere/else"),
            &f.env.usr_local_bin,
        ])
        .unwrap()
        .into_string()
        .unwrap();
        let report = install(&f.env, false).unwrap();
        assert!(report.path_hint.is_none());
    }

    #[test]
    fn missing_bundled_binary_is_an_error() {
        let f = Fixture::new("missing");
        std::fs::remove_file(f.env.exe_dir.join(BUNDLED_CLI_NAME)).unwrap();
        let err = install(&f.env, false).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn bundled_cli_that_is_the_app_binary_is_an_error() {
        // Simulates the case-insensitive-filesystem collision where the
        // "sidecar" path resolves to the app binary itself.
        let mut f = Fixture::new("collision");
        f.env.exe_path = f.env.exe_dir.join(BUNDLED_CLI_NAME);
        let err = install(&f.env, false).unwrap_err();
        assert!(err.contains("app binary itself"));
        assert!(!f.env.usr_local_bin.join(CLI_NAME).exists());
    }
}
