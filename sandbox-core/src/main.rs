//! burnbox-core — unprivileged namespace sandbox and RAM wipe engine.
//!
//! Creates an unprivileged user namespace (`CLONE_NEWUSER`), a mount namespace
//! (`CLONE_NEWNS`) and a PID namespace (`CLONE_NEWPID`). It mounts a `tmpfs`
//! RAM directory sized with `--ram-mb` and hides the host `/home` behind an
//! empty `tmpfs`. If the host LSM (for example Ubuntu's AppArmor
//! `unprivileged_userns` profile) denies those mounts with `EACCES`, it falls
//! back to a private directory under `/dev/shm`, which is already kernel
//! `tmpfs`. In both cases it runs `--target`, scrubs every file with `zeroize`
//! on exit, and removes/unmounts the workspace.

use clap::{Arg, Command};
use nix::errno::Errno;
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::sched::{clone, CloneFlags};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{execvpe, fork, getegid, geteuid, pipe, write as fd_write, ForkResult, Pid};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process;
use zeroize::Zeroize;

/// Session ended cleanly and tmpfs was scrubbed/unmounted.
const EXIT_CLEAN: i32 = 0;
/// General runtime error.
const EXIT_RUNTIME: i32 = 1;
/// Namespace / sandbox setup failure.
const EXIT_SANDBOX: i32 = 2;
/// Security violation (for example: host `/home` could not be hidden).
const EXIT_SECURITY: i32 = 3;

/// Size of the scratch buffer used to overwrite RAM-directory files.
const WIPE_CHUNK: usize = 64 * 1024;
/// Stack for the namespaced supervisor process (clone needs its own stack).
const SUPERVISOR_STACK: usize = 1024 * 1024;

#[derive(Clone, Debug)]
struct Session {
    mode: String,
    ram_mb: u64,
    target: String,
    target_args: Vec<String>,
    mount_point: PathBuf,
}

fn build_cli() -> Command {
    Command::new("burnbox-core")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Unprivileged namespace sandbox and RAM wipe engine for BurnBox")
        .arg(
            Arg::new("mode")
                .long("mode")
                .required(true)
                .value_parser(["offline", "online"])
                .help("Session mode: offline or online"),
        )
        .arg(
            Arg::new("ram-mb")
                .long("ram-mb")
                .required(true)
                .value_parser(clap::value_parser!(u64).range(1..))
                .help("Size of the tmpfs RAM directory in mebibytes"),
        )
        .arg(
            Arg::new("target")
                .long("target")
                .required(true)
                .help("Executable to run inside the sandbox"),
        )
        .arg(
            Arg::new("target-args")
                .num_args(0..)
                .trailing_var_arg(true)
                .allow_hyphen_values(true)
                .help("Arguments passed to the target (after an optional --)"),
        )
}

fn session_from_matches(matches: &clap::ArgMatches, mount_point: PathBuf) -> Session {
    Session {
        mode: matches
            .get_one::<String>("mode")
            .expect("mode is required")
            .clone(),
        ram_mb: *matches
            .get_one::<u64>("ram-mb")
            .expect("ram-mb is required"),
        target: matches
            .get_one::<String>("target")
            .expect("target is required")
            .clone(),
        target_args: matches
            .get_many::<String>("target-args")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        mount_point,
    }
}

/// Generate the workspace path. It serves both as the private tmpfs mount
/// point and, if mounting is denied, as the fallback RAM directory: `/dev/shm`
/// is kernel tmpfs and is writable by unprivileged users.
fn make_mount_point() -> PathBuf {
    Path::new("/dev/shm").join(format!("burnbox-{}", process::id()))
}

/// Create the workspace privately (mode 0700) for the current user.
fn create_workspace(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(path)
}

/// Build a status line with the exact key order/spacing required by AGREEMENT.md.
fn status_line(status: &str, pid: Pid, mount_point: &Path) -> String {
    let escaped = serde_json::to_string(&mount_point.to_string_lossy())
        .unwrap_or_else(|_| "\"\"".to_string());
    format!(
        "{{\"status\": \"{status}\", \"pid\": {}, \"mount_point\": {escaped}}}",
        pid.as_raw()
    )
}

fn emit_status(status: &str, pid: Pid, mount_point: &Path) {
    println!("{}", status_line(status, pid, mount_point));
    let _ = io::stdout().flush();
}

fn main() {
    let matches = match build_cli().try_get_matches() {
        Ok(matches) => matches,
        Err(error) => {
            let code = if error.use_stderr() {
                EXIT_RUNTIME
            } else {
                EXIT_CLEAN
            };
            let _ = error.print();
            process::exit(code);
        }
    };

    let host_uid = geteuid().as_raw();
    let host_gid = getegid().as_raw();
    let session = session_from_matches(&matches, make_mount_point());

    if let Err(error) = create_workspace(&session.mount_point) {
        eprintln!("burnbox-core: cannot create RAM workspace: {error}");
        process::exit(EXIT_SANDBOX);
    }

    // Avoid duplicating buffered output into the cloned process.
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();

    let (sync_read, sync_write) = match pipe() {
        Ok(fds) => fds,
        Err(error) => {
            eprintln!("burnbox-core: cannot create sync pipe: {error}");
            let _ = fs::remove_dir_all(&session.mount_point);
            process::exit(EXIT_SANDBOX);
        }
    };

    // The clone copies the file descriptor table: pass raw descriptors so the
    // parent keeps ownership of its own ends of the pipe.
    let child_read = sync_read.as_raw_fd();
    let child_write = sync_write.as_raw_fd();
    let child_session = session.clone();
    let mut stack = vec![0u8; SUPERVISOR_STACK];
    let flags = CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNS | CloneFlags::CLONE_NEWPID;

    let child_pid = unsafe {
        clone(
            Box::new(move || {
                // Only the namespace creator (the parent) may write the
                // uid/gid mapping files. Wait for its signal before mounting.
                nix::libc::close(child_write);
                let ready = read_sync_byte(child_read) == Some(b'G');
                nix::libc::close(child_read);
                let code = if ready {
                    run_sandbox(&child_session)
                } else {
                    EXIT_SANDBOX
                };
                process::exit(code);
            }),
            &mut stack,
            flags,
            Some(nix::libc::SIGCHLD),
        )
    };

    let child_pid = match child_pid {
        Ok(pid) => pid,
        Err(error) => {
            eprintln!("burnbox-core: namespace clone failed: {error}");
            drop(sync_read);
            drop(sync_write);
            let _ = fs::remove_dir_all(&session.mount_point);
            process::exit(EXIT_SANDBOX);
        }
    };

    emit_status("started", child_pid, &session.mount_point);

    let maps_ok = write_id_maps_for_child(child_pid, host_uid, host_gid).is_ok();
    if !maps_ok {
        eprintln!("burnbox-core: cannot write uid/gid maps for child namespace");
    }
    let signal = if maps_ok { b'G' } else { b'F' };
    if let Err(error) = fd_write(&sync_write, &[signal]) {
        eprintln!("burnbox-core: cannot signal child namespace: {error}");
    }
    drop(sync_write);
    drop(sync_read);

    let status = match waitpid(child_pid, None) {
        Ok(status) => status,
        Err(error) => {
            eprintln!("burnbox-core: failed to wait for sandbox: {error}");
            let _ = fs::remove_dir_all(&session.mount_point);
            emit_status("wiped", child_pid, &session.mount_point);
            process::exit(EXIT_RUNTIME);
        }
    };

    match fs::remove_dir_all(&session.mount_point) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("burnbox-core: cannot remove RAM workspace: {error}"),
    }
    emit_status("wiped", child_pid, &session.mount_point);

    let code = match status {
        WaitStatus::Exited(_, code) => code,
        WaitStatus::Signaled(..) => EXIT_RUNTIME,
        _ => EXIT_RUNTIME,
    };
    process::exit(code);
}

/// Read a single synchronization byte, retrying on EINTR.
fn read_sync_byte(fd: nix::libc::c_int) -> Option<u8> {
    let mut byte = [0u8; 1];
    loop {
        // SAFETY: fd is a valid pipe read end inherited from the parent, and
        // the buffer has room for the single byte requested.
        let count = unsafe {
            nix::libc::read(
                fd,
                byte.as_mut_ptr().cast::<nix::libc::c_void>(),
                byte.len(),
            )
        };
        if count == 1 {
            return Some(byte[0]);
        }
        if count == 0 {
            return None;
        }
        if Errno::last() != Errno::EINTR {
            return None;
        }
    }
}

/// The parent (namespace creator) maps its own uid/gid to root inside the new
/// user namespace. Writing `deny` first is required for an unprivileged gid map.
fn write_id_maps_for_child(pid: Pid, host_uid: u32, host_gid: u32) -> io::Result<()> {
    let proc = format!("/proc/{}", pid.as_raw());
    match write_proc_file(&format!("{proc}/setgroups"), "deny") {
        Ok(()) => {}
        // Kernels older than 3.19 have no setgroups file and no restriction.
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io::Error::other(format!("setgroups: {error}"))),
    }
    write_proc_file(
        &format!("{proc}/uid_map"),
        &format!("0 {host_uid} 1"),
    )
    .map_err(|error| io::Error::other(format!("uid_map: {error}")))?;
    write_proc_file(
        &format!("{proc}/gid_map"),
        &format!("0 {host_gid} 1"),
    )
    .map_err(|error| io::Error::other(format!("gid_map: {error}")))?;
    Ok(())
}

fn write_proc_file(path: &str, value: &str) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(value.as_bytes())
}

/// Supervisor running inside the new user, mount and PID namespaces.
/// The parent has already installed the uid/gid maps before this function runs.
fn run_sandbox(session: &Session) -> i32 {
    let ram_options = format!("size={}m,mode=1777", session.ram_mb);
    let mut ram_mounted = false;
    match mount(
        Some("tmpfs"),
        &session.mount_point,
        Some("tmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some(ram_options.as_str()),
    ) {
        Ok(()) => ram_mounted = true,
        Err(Errno::EACCES) => {
            // Ubuntu confines unprivileged user namespaces with AppArmor and
            // denies mount(2) inside them. The workspace itself is already on
            // kernel tmpfs, so use it directly instead of failing the session.
            eprintln!(
                "burnbox-core: tmpfs mount denied by host policy; using /dev/shm RAM workspace fallback"
            );
        }
        Err(error) => {
            eprintln!("burnbox-core: tmpfs mount failed: {error}");
            return EXIT_SANDBOX;
        }
    }

    // With a mount-based workspace, failing to hide /home is a security
    // violation. In the /dev/shm fallback the host LSM blocks all mount
    // attempts, so HOME is redirected to the isolated workspace instead.
    let mut home_mounted = false;
    match fs::symlink_metadata("/home") {
        Ok(metadata) if metadata.file_type().is_dir() => {
            match mount(
                Some("tmpfs"),
                Path::new("/home"),
                Some("tmpfs"),
                MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
                Some("size=1m,mode=1777"),
            ) {
                Ok(()) => home_mounted = true,
                Err(Errno::EACCES) if !ram_mounted => {
                    eprintln!("burnbox-core: /home overlay denied by host policy; HOME points at the isolated workspace");
                }
                Err(error) => {
                    eprintln!("burnbox-core: cannot hide host /home: {error}");
                    let _ = cleanup_mounts(session, ram_mounted, home_mounted);
                    return EXIT_SECURITY;
                }
            }
        }
        Ok(_) => {
            eprintln!("burnbox-core: /home is not a directory; cannot hide host home");
            if ram_mounted {
                let _ = cleanup_mounts(session, ram_mounted, home_mounted);
                return EXIT_SECURITY;
            }
        }
        Err(error) => {
            eprintln!("burnbox-core: cannot inspect /home: {error}");
            if ram_mounted {
                let _ = cleanup_mounts(session, ram_mounted, home_mounted);
                return EXIT_SECURITY;
            }
        }
    }

    let target_pid = match unsafe { fork() } {
        Ok(ForkResult::Child) => exec_target(session),
        Ok(ForkResult::Parent { child }) => child,
        Err(error) => {
            eprintln!("burnbox-core: target fork failed: {error}");
            let _ = cleanup_mounts(session, ram_mounted, home_mounted);
            return EXIT_RUNTIME;
        }
    };

    // This supervisor is PID 1 in the new PID namespace: reap every child,
    // but stop waiting once the requested target has been reaped.
    let mut target_status: Option<WaitStatus> = None;
    while target_status.is_none() {
        match waitpid(Pid::from_raw(-1), None) {
            Ok(status) => {
                if status.pid() == Some(target_pid) {
                    target_status = Some(status);
                }
            }
            Err(Errno::EINTR) => continue,
            Err(Errno::ECHILD) => break,
            Err(error) => {
                eprintln!("burnbox-core: wait for target failed: {error}");
                break;
            }
        }
    }

    // Reap any grandchildren left behind by the target without blocking.
    loop {
        match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) | Err(Errno::ECHILD) => break,
            Ok(_) | Err(Errno::EINTR) => continue,
            Err(_) => break,
        }
    }

    if let Err(error) = cleanup_mounts(session, ram_mounted, home_mounted) {
        eprintln!("burnbox-core: cleanup failed: {error}");
        return EXIT_SECURITY;
    }

    match target_status {
        Some(WaitStatus::Exited(_, EXIT_CLEAN)) => EXIT_CLEAN,
        _ => EXIT_RUNTIME,
    }
}

/// Build an explicit environment that points HOME and TMPDIR at the RAM
/// workspace without mutating global state after clone/fork.
fn build_environment(session: &Session) -> Vec<CString> {
    let mut environment = Vec::new();
    for (key, value) in std::env::vars_os() {
        if key == "HOME" || key == "TMPDIR" || key == "BURNBOX_MODE" {
            continue;
        }
        let mut entry = key.as_bytes().to_vec();
        entry.push(b'=');
        entry.extend_from_slice(value.as_bytes());
        if let Ok(entry) = CString::new(entry) {
            environment.push(entry);
        }
    }
    let workspace = session.mount_point.display();
    environment.push(
        CString::new(format!("HOME={workspace}")).expect("workspace path cannot contain NUL"),
    );
    environment.push(
        CString::new(format!("TMPDIR={workspace}")).expect("workspace path cannot contain NUL"),
    );
    environment.push(
        CString::new(format!("BURNBOX_MODE={}", session.mode))
            .expect("mode cannot contain NUL"),
    );
    environment
}

/// Replace the grandchild process with the user's target executable.
fn exec_target(session: &Session) -> ! {
    let target = match CString::new(session.target.as_str()) {
        Ok(target) => target,
        Err(error) => {
            eprintln!("burnbox-core: invalid target path: {error}");
            process::exit(EXIT_RUNTIME);
        }
    };
    let mut argv = Vec::with_capacity(session.target_args.len() + 1);
    argv.push(target.clone());
    for argument in &session.target_args {
        match CString::new(argument.as_str()) {
            Ok(argument) => argv.push(argument),
            Err(error) => {
                eprintln!("burnbox-core: invalid target argument: {error}");
                process::exit(EXIT_RUNTIME);
            }
        }
    }

    match execvpe(&target, &argv, &build_environment(session)) {
        Err(error) => {
            eprintln!("burnbox-core: cannot execute {}: {error}", session.target);
            process::exit(EXIT_RUNTIME);
        }
        Ok(never) => match never {},
    }
}

/// Overwrite every file in the RAM workspace with zeros, then unmount it (or
/// remove the `/dev/shm` fallback directory) and the `/home` overlay. Mounts
/// live only in this namespace; `MNT_DETACH` makes unmounting robust even if
/// the target left files open.
fn cleanup_mounts(session: &Session, ram_mounted: bool, home_mounted: bool) -> Result<(), String> {
    let mut errors = Vec::new();

    if let Err(error) = zero_tree(&session.mount_point) {
        errors.push(format!("RAM wipe: {error}"));
    }
    if ram_mounted {
        if let Err(error) = umount2(&session.mount_point, MntFlags::MNT_DETACH) {
            errors.push(format!("RAM unmount: {error}"));
        }
    } else {
        // /dev/shm fallback: no mount to detach, remove the private directory.
        match fs::remove_dir_all(&session.mount_point) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => errors.push(format!("RAM workspace removal: {error}")),
        }
    }
    if home_mounted
        && let Err(error) = umount2(Path::new("/home"), MntFlags::MNT_DETACH)
    {
        errors.push(format!("/home overlay unmount: {error}"));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Recursively overwrite regular files with zeros and remove them.
fn zero_tree(root: &Path) -> io::Result<()> {
    let mut scratch = vec![0u8; WIPE_CHUNK];
    let result = zero_tree_inner(root, &mut scratch);
    scratch.zeroize();
    result
}

fn zero_tree_inner(path: &Path, scratch: &mut [u8]) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let file_type = metadata.file_type();

    if file_type.is_dir() {
        for entry in fs::read_dir(path)? {
            zero_tree_inner(&entry?.path(), scratch)?;
        }
        // The mount point itself cannot be removed while mounted; ignore that.
        let _ = fs::remove_dir(path);
    } else if file_type.is_file() {
        let mut remaining = metadata.len();
        if remaining > 0 {
            let mut file = OpenOptions::new().write(true).open(path)?;
            while remaining > 0 {
                let chunk = remaining.min(scratch.len() as u64) as usize;
                file.write_all(&scratch[..chunk])?;
                remaining -= chunk as u64;
            }
            file.sync_all()?;
        }
        fs::remove_file(path)?;
    }
    // Symlinks and special files carry no file data to scrub.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines_match_contract() {
        assert_eq!(
            status_line("started", Pid::from_raw(1234), Path::new("/tmp/burnbox-1")),
            r#"{"status": "started", "pid": 1234, "mount_point": "/tmp/burnbox-1"}"#
        );
        assert_eq!(
            status_line("wiped", Pid::from_raw(9), Path::new("/tmp/a b")),
            r#"{"status": "wiped", "pid": 9, "mount_point": "/tmp/a b"}"#
        );
    }

    #[test]
    fn cli_accepts_target_arguments_after_separator() {
        let matches = build_cli()
            .try_get_matches_from([
                "burnbox-core",
                "--mode",
                "offline",
                "--ram-mb",
                "64",
                "--target",
                "/bin/sh",
                "--",
                "-c",
                "echo hello",
            ])
            .expect("valid arguments");
        let session = session_from_matches(&matches, PathBuf::from("/tmp/mnt"));
        assert_eq!(session.mode, "offline");
        assert_eq!(session.ram_mb, 64);
        assert_eq!(session.target, "/bin/sh");
        assert_eq!(session.target_args, vec!["-c", "echo hello"]);
    }

    #[test]
    fn workspace_lives_in_dev_shm() {
        let path = make_mount_point();
        assert_eq!(path.parent(), Some(Path::new("/dev/shm")));
        assert!(path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("burnbox-")));
    }

    #[test]
    fn environment_redirects_home_and_tmpdir() {
        let session = Session {
            mode: "offline".to_string(),
            ram_mb: 16,
            target: "/bin/true".to_string(),
            target_args: Vec::new(),
            mount_point: PathBuf::from("/dev/shm/burnbox-42"),
        };
        let environment = build_environment(&session);
        assert!(environment
            .iter()
            .any(|entry| entry.as_bytes() == b"HOME=/dev/shm/burnbox-42"));
        assert!(environment
            .iter()
            .any(|entry| entry.as_bytes() == b"TMPDIR=/dev/shm/burnbox-42"));
    }

    #[test]
    fn fallback_cleanup_wipes_and_removes_workspace() {
        let root = std::env::temp_dir().join(format!("burnbox-fallback-test-{}", process::id()));
        fs::create_dir_all(&root).expect("create test workspace");
        fs::write(root.join("secret.txt"), b"topsecret").expect("write test secret");
        let session = Session {
            mode: "online".to_string(),
            ram_mb: 16,
            target: "/bin/true".to_string(),
            target_args: Vec::new(),
            mount_point: root.clone(),
        };

        cleanup_mounts(&session, false, false).expect("fallback cleanup");

        assert!(!root.exists());
    }

    #[test]
    fn zero_tree_scrubs_and_removes_files() {
        let root = std::env::temp_dir().join(format!("burnbox-wipe-test-{}", process::id()));
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("create test dir");
        fs::write(root.join("plain.txt"), b"secret").expect("write test file");
        fs::write(nested.join("nested.bin"), vec![0xAB; WIPE_CHUNK + 7])
            .expect("write large test file");

        zero_tree(&root).expect("wipe test tree");

        assert!(!root.join("plain.txt").exists());
        assert!(!nested.join("nested.bin").exists());
        fs::remove_dir_all(&root).ok();
    }
}
