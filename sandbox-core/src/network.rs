//! Rootless user-space networking for `online` sessions.
//!
//! When `slirp4netns` is available it is launched from the host side against
//! the supervisor's network namespace (addressed by the supervisor's host
//! PID). It creates and configures a TAP device (`burnbox0`, 10.0.2.100/24)
//! inside that namespace, so the target gets user-mode TCP/IP without needing
//! a veth pair, a bridge, or any host privileges.
//!
//! Readiness is synchronized through `--ready-fd=1`: slirp4netns writes one
//! byte to its stdout once the interface is up, and the parent waits for that
//! byte before releasing the supervisor to exec the target. This guarantees
//! the target never observes a half-configured network namespace.
//!
//! If the binary is missing (or fails to start), online sessions keep the
//! previous loopback-only behavior and emit an explicit stderr warning.
//! `offline` sessions never call into this module: their isolation assertion
//! would reject the external interface.

use nix::errno::Errno;
use nix::poll::{poll, PollFd, PollFlags};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Exact stderr warning emitted when slirp4netns cannot be found.
pub const SLIRP_MISSING_WARNING: &str = "Warning: slirp4netns not found; network isolation is strict loopback only. Install slirp4netns for live URL fetching.";

/// Interface created inside the sandbox network namespace.
pub const TAP_NAME: &str = "burnbox0";

/// Optional override for packagers and tests that install slirp4netns off PATH.
pub const SLIRP_OVERRIDE_ENV: &str = "BURNBOX_SLIRP4NETNS";

/// Maximum time to wait for slirp4netns to finish configuring the tap device.
const READY_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether a path is a regular file with at least one execute bit set.
fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Search a PATH-like string for an executable named `name`.
///
/// `path` is injectable so the lookup can be unit tested without mutating the
/// process-wide PATH (which would race with parallel tests).
pub fn find_in_path(name: &OsStr, path: Option<&OsStr>) -> Option<PathBuf> {
    let path: OsString = match path {
        Some(value) => value.to_os_string(),
        None => std::env::var_os("PATH")?,
    };
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

/// Locate the slirp4netns binary, honoring the optional path override.
pub fn find_slirp4netns() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(SLIRP_OVERRIDE_ENV) {
        let path = PathBuf::from(value);
        if is_executable(&path) {
            return Some(path);
        }
        eprintln!(
            "burnbox-core: warning: {SLIRP_OVERRIDE_ENV}={} is not executable; ignoring",
            path.display()
        );
    }
    find_in_path(OsStr::new("slirp4netns"), None)
}

/// A running slirp4netns helper. Killing it on drop tears down the tap device
/// with the namespace, so no host state survives the session.
pub struct Slirp {
    child: Child,
}

impl Slirp {
    /// Wait for the single readiness byte on slirp4netns's stdout.
    pub fn wait_ready(&mut self) -> Result<(), String> {
        let stdout = self
            .child
            .stdout
            .as_mut()
            .ok_or("slirp4netns stdout was not piped")?;
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("timed out waiting for slirp4netns readiness".to_string());
            }
            // Poll in short slices so the deadline is rechecked without
            // relying on a platform-specific maximum timeout width.
            let timeout_ms: u16 = remaining
                .as_millis()
                .min(100)
                .try_into()
                .expect("poll slice fits in u16");
            let mut fds = [PollFd::new(stdout.as_fd(), PollFlags::POLLIN)];
            match poll(&mut fds, timeout_ms) {
                Ok(0) => continue,
                Ok(_) => {}
                Err(Errno::EINTR) => continue,
                Err(error) => return Err(format!("poll ready fd: {error}")),
            }
            let mut byte = [0u8; 1];
            match stdout.read(&mut byte) {
                Ok(0) => {
                    return Err("slirp4netns closed ready fd before configuring the tap".to_string());
                }
                Ok(_) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(format!("read ready fd: {error}")),
            }
        }
    }
}

impl Drop for Slirp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawn slirp4netns against the network namespace of `target_pid`.
///
/// `--configure` brings `burnbox0` up with 10.0.2.100/24 and the default
/// slirp gateway/DNS; `--ready-fd=1` lets the parent synchronize before exec.
pub fn start(target_pid: i32, binary: &Path) -> Result<Slirp, String> {
    let child = Command::new(binary)
        .args([
            "--configure",
            "--ready-fd=1",
            &target_pid.to_string(),
            TAP_NAME,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("spawn {}: {error}", binary.display()))?;
    Ok(Slirp { child })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::Permissions;
    use std::path::Path;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(name: &str) -> PathBuf {
        // No ':' in the name: PATH is colon-separated on Unix.
        let path = std::env::temp_dir().join(format!(
            "burnbox-net-{name}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn find_in_path_finds_an_executable() {
        let dir = temp_dir("find");
        let binary = dir.join("slirp4netns");
        fs::write(&binary, "#!/bin/sh\n").expect("write stub");
        fs::set_permissions(&binary, Permissions::from_mode(0o755)).expect("chmod");
        let path = OsStr::new(&dir);
        assert_eq!(find_in_path(OsStr::new("slirp4netns"), Some(path)), Some(binary));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn find_in_path_ignores_non_executables_and_missing_entries() {
        let dir = temp_dir("non-exec");
        let binary = dir.join("slirp4netns");
        fs::write(&binary, "#!/bin/sh\n").expect("write stub");
        fs::set_permissions(&binary, Permissions::from_mode(0o644)).expect("chmod");
        let path = OsStr::new(&dir);
        assert_eq!(find_in_path(OsStr::new("slirp4netns"), Some(path)), None);
        assert_eq!(find_in_path(OsStr::new("absent"), Some(path)), None);
        assert_eq!(find_in_path(OsStr::new("slirp4netns"), Some(OsStr::new("/nonexistent"))), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_warning_matches_the_required_text() {
        assert_eq!(
            SLIRP_MISSING_WARNING,
            "Warning: slirp4netns not found; network isolation is strict loopback only. Install slirp4netns for live URL fetching."
        );
        assert_eq!(TAP_NAME, "burnbox0");
        assert!(Path::new(TAP_NAME).is_relative());
    }
}
