//! Optional read-only filesystem jail built with a private `pivot_root`.
//!
//! This module is opt-in (`--fs-jail`). It builds a minimal root filesystem
//! from a fresh tmpfs, bind-mounts only the loader/binaries it needs read-only,
//! provides fresh `/dev`, `/tmp` and `/proc` (with `hidepid=2,gid=0` so host
//! processes cannot be enumerated), and bind-mounts the RAM workspace at
//! `/sandbox`. The supervisor then pivots into it **before** forking the
//! target, so host `/etc`, `/var`, `/run` and the host `/proc` are unreachable.
//!
//! Mounting inside an unprivileged user namespace is denied by some host LSMs
//! (notably Ubuntu's AppArmor `unprivileged_userns` profile). In that case
//! setup returns an error and the caller continues with the pre-existing
//! overlay behavior and a stderr warning: the jail is never a hard failure.

use nix::mount::{umount2, MntFlags, MsFlags};
use nix::unistd::pivot_root;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

/// Where the RAM workspace appears inside the jail.
pub const SANDBOX_PATH: &str = "/sandbox";
/// Scratch mount used as the old root during `pivot_root`.
pub const OLD_ROOT: &str = "/.old_root";
/// Procfs options: fresh PID namespace plus hidden processes for other users.
pub const PROC_OPTIONS: &str = "hidepid=2,gid=0";
/// Subdirectories created in the fresh root before mounts are applied.
pub const JAIL_DIRS: &[&str] = &[
    "usr/bin",
    "lib",
    "lib64",
    "bin",
    "dev",
    "tmp",
    "proc",
    "sandbox",
    ".old_root",
];
/// Host directories exposed read-only for the dynamic loader and binaries.
pub const BIND_SOURCES: &[&str] = &["/usr", "/lib", "/lib64", "/bin"];

/// Detect the `AppImage` runtime mount prefix (for example `/tmp/.mount_AB12cd`)
/// in an arbitrary path. `AppImage` extracts its payload there and may place
/// viewers and their libraries under it, so a jail built only from host
/// `/usr`/`/lib` would hide the executable the target actually needs.
pub fn appimage_mount_from_path(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|ancestor| {
        let name = ancestor.file_name()?.to_str()?;
        name.starts_with(".mount_")
            .then(|| ancestor.to_path_buf())
    })
}

/// Detect an AppImage runtime from the requested target, `APPDIR` or the path
/// of this executable. Used to keep `--fs-jail` from breaking abstracted host
/// filesystem layouts.
pub fn appimage_mount_point(target: &Path) -> Option<PathBuf> {
    if let Some(mount) = appimage_mount_from_path(target) {
        return Some(mount);
    }
    if let Some(appdir) = std::env::var_os("APPDIR")
        && let Some(mount) = appimage_mount_from_path(Path::new(&appdir))
    {
        return Some(mount);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| appimage_mount_from_path(&exe))
}

/// Host-side path of the jail's tmpfs root for a given supervisor PID.
pub fn root_path_for(pid: i32) -> PathBuf {
    Path::new("/dev/shm").join(format!("burnbox-jail-{pid}"))
}

/// Host-side path of this supervisor's jail root.
pub fn root_path() -> PathBuf {
    root_path_for(process::id() as i32)
}

/// A successfully pivoted jail. `mounted` contains jail-relative paths so the
/// same list can be detached before the pivot (under `root`) or after it
/// (under `/`).
pub struct Jail {
    root: PathBuf,
    mounted: Vec<&'static str>,
    active: bool,
}

impl Jail {
    pub fn is_active(&self) -> bool {
        self.active
    }

    fn remember(&mut self, relative: &'static str) {
        if !self.mounted.contains(&relative) {
            self.mounted.push(relative);
        }
    }

    /// Mount `source` (or a filesystem of `fstype`) at `root/<relative>`.
    fn add_mount(
        &mut self,
        source: Option<&str>,
        relative: &'static str,
        fstype: Option<&str>,
        flags: MsFlags,
        data: Option<&str>,
        stage: &str,
    ) -> Result<(), String> {
        let target = self.root.join(relative);
        nix::mount::mount::<str, Path, str, str>(source, &target, fstype, flags, data)
            .map_err(|error| format!("{stage}: {error}"))?;
        self.remember(relative);
        Ok(())
    }

    /// Turn a completed bind mount read-only without changing its source.
    fn seal_read_only(&mut self, relative: &'static str, stage: &str) -> Result<(), String> {
        let target = self.root.join(relative);
        nix::mount::mount::<str, Path, str, str>(
            None,
            &target,
            None,
            MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY,
            None,
        )
        .map_err(|error| format!("{stage}: {error}"))
    }

    /// Build the jail and pivot into it. On any failure the partial mounts are
    /// detached and the caller falls back to the overlay behavior.
    pub fn setup(workspace: &Path, ram_mb: u64, target: &Path) -> Result<Jail, String> {
        // AppImage payloads live under /tmp/.mount_* and are not covered by the
        // static /usr /lib /lib64 /bin bind list. Refuse the jail so the caller
        // can fall back to the overlay with a warning instead of hiding the
        // viewer's own libraries.
        if let Some(mount) = appimage_mount_point(target) {
            return Err(format!(
                "AppImage mount {} needs the overlay filesystem view",
                mount.display()
            ));
        }

        let root = root_path();
        let _ = fs::remove_dir_all(&root);
        for dir in JAIL_DIRS {
            fs::create_dir_all(root.join(dir))
                .map_err(|error| format!("create jail directory {dir}: {error}"))?;
        }

        let mut jail = Jail {
            root,
            mounted: Vec::new(),
            active: false,
        };
        let root_options = format!("size={}m,mode=1777", ram_mb.max(16));
        if let Err(error) = jail.add_mount(
            None,
            "",
            Some("tmpfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
            Some(&root_options),
            "jail root tmpfs",
        ) {
            jail.abort();
            return Err(error);
        }

        for source in BIND_SOURCES {
            if !Path::new(source).exists() {
                continue;
            }
            let relative: &'static str = match *source {
                "/usr" => "usr",
                "/lib" => "lib",
                "/lib64" => "lib64",
                "/bin" => "bin",
                // Every entry of BIND_SOURCES is one of the literals above.
                _ => unreachable!("unhandled bind source"),
            };
            if let Err(error) = jail.add_mount(
                Some(source),
                relative,
                None,
                MsFlags::MS_BIND,
                None,
                &format!("bind {source}"),
            ) {
                jail.abort();
                return Err(error);
            }
            if let Err(error) = jail.seal_read_only(relative, &format!("seal {source}")) {
                jail.abort();
                return Err(error);
            }
        }

        let workspace_source = workspace
            .to_str()
            .ok_or_else(|| "workspace path is not valid UTF-8".to_string())?;
        let results = [
            jail.add_mount(
                None,
                "dev",
                Some("tmpfs"),
                MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
                Some("size=1m,mode=1777"),
                "fresh /dev",
            ),
            jail.add_mount(
                None,
                "tmp",
                Some("tmpfs"),
                MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
                Some("size=16m,mode=1777"),
                "fresh /tmp",
            ),
            jail.add_mount(
                None,
                "proc",
                Some("proc"),
                MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
                Some(PROC_OPTIONS),
                "fresh /proc",
            ),
            jail.add_mount(
                Some(workspace_source),
                "sandbox",
                None,
                MsFlags::MS_BIND,
                None,
                "bind workspace at /sandbox",
            ),
        ];
        for result in results {
            if let Err(error) = result {
                jail.abort();
                return Err(error);
            }
        }

        let old_root = jail.root.join(".old_root");
        if let Err(error) = pivot_root(&jail.root, &old_root) {
            jail.abort();
            return Err(format!("pivot_root: {error}"));
        }
        jail.active = true;
        let _ = umount2(Path::new(OLD_ROOT), MntFlags::MNT_DETACH);
        Ok(jail)
    }

    /// Detach a partially built jail before the pivot.
    fn abort(&self) {
        for relative in self.mounted.iter().rev() {
            let _ = umount2(&self.root.join(relative), MntFlags::MNT_DETACH);
        }
        let _ = fs::remove_dir_all(&self.root);
    }

    /// Detach the jail's mounts after the target exits. The tmpfs root itself
    /// disappears with the private mount namespace, so it is left in place.
    pub fn finish(&self) {
        if !self.active {
            return;
        }
        for relative in self.mounted.iter().rev() {
            if relative.is_empty() {
                continue; // current root
            }
            let _ = umount2(&Path::new("/").join(relative), MntFlags::MNT_DETACH);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jail_exposes_only_loader_directories() {
        for expected in ["/usr", "/lib", "/lib64", "/bin"] {
            assert!(BIND_SOURCES.contains(&expected), "missing bind source {expected}");
        }
        assert!(!BIND_SOURCES.contains(&"/etc"), "host /etc must never be bound");
        assert!(!BIND_SOURCES.contains(&"/var"));
        assert!(!BIND_SOURCES.contains(&"/run"));
    }

    #[test]
    fn proc_is_fresh_and_hidden() {
        assert_eq!(PROC_OPTIONS, "hidepid=2,gid=0");
        assert!(JAIL_DIRS.contains(&"proc"));
        assert!(JAIL_DIRS.contains(&"sandbox"));
    }

    #[test]
    fn appimage_mount_is_detected_from_target_paths() {
        let mount = Path::new("/tmp/.mount_AB12cd");
        let viewer = mount.join("usr/bin/firefox");
        assert_eq!(appimage_mount_from_path(&viewer), Some(mount.to_path_buf()));
        assert_eq!(
            appimage_mount_from_path(mount),
            Some(mount.to_path_buf())
        );
        assert_eq!(appimage_mount_from_path(Path::new("/usr/bin/firefox")), None);
        // A directory merely named `mount_*` is not an AppImage runtime.
        assert_eq!(
            appimage_mount_from_path(Path::new("/tmp/mount_AB12cd/usr/bin/firefox")),
            None
        );
    }

    #[test]
    fn appimage_payload_is_not_in_the_static_bind_list() {
        assert!(
            BIND_SOURCES
                .iter()
                .all(|source| !source.starts_with("/tmp/.mount_")),
            "static bind list must not pretend to cover AppImage payload paths"
        );
    }

    #[test]
    fn jail_paths_are_derived_from_supervisor_pid() {
        let root = root_path_for(4242);
        assert_eq!(root, Path::new("/dev/shm/burnbox-jail-4242"));
        assert_eq!(SANDBOX_PATH, "/sandbox");
        assert_eq!(OLD_ROOT, "/.old_root");
    }
}
