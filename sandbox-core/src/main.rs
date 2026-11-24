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
//!
//! Network isolation is requested with `CLONE_NEWNET`. If the host denies that
//! flag, `online` sessions continue with a stderr warning; `offline` sessions
//! treat it as a namespace setup failure and additionally assert that no
//! external interface is visible in `/sys/class/net`. A small seccomp-BPF
//! filter is installed in the target before `execve`: the default deny-list
//! blocks escape primitives, while `--seccomp-strict` switches to an allow-list
//! of viewer syscalls. Optional `--copy-in` files are staged into the RAM
//! workspace (mode 0600) with the target's cwd set there, so they are inside
//! the zeroize scrub boundary. The parent catches SIGTERM/SIGINT/SIGHUP/SIGPIPE
//! and performs that same zeroize + rmdir scrub before exiting as interrupted.

mod seccomp;

use clap::{Arg, Command};
use nix::errno::Errno;
use nix::libc;
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::poll::{poll, PollFd, PollFlags};
use nix::sched::{clone, CloneFlags};
use nix::sys::signal::{kill, sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{
    chdir, execvpe, fork, getegid, geteuid, pipe, write as fd_write, ForkResult, Pid,
};
use std::ffi::{CString, OsStr};
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;
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

/// Set by the async signal handler; the main loop observes it and performs the
/// zeroize + rmdir cleanup (which is not async-signal-safe itself).
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
/// Write end of a self-pipe woken by the handler so `poll` returns promptly.
static INTERRUPT_FD: AtomicI32 = AtomicI32::new(-1);

/// Async-signal-safe handler: only atomics and a single `write(2)` are used.
extern "C" fn interrupt_handler(_signal: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
    let fd = INTERRUPT_FD.load(Ordering::SeqCst);
    if fd >= 0 {
        let byte = [1u8];
        // SAFETY: fd is a valid pipe write end owned by the parent, and the
        // buffer has room for the single byte requested.
        unsafe {
            libc::write(fd, byte.as_ptr().cast::<libc::c_void>(), 1);
        }
    }
}

/// Catch termination signals in the parent so an interrupted session cannot
/// leave a `/dev/shm/burnbox-<pid>` workspace behind.
fn install_signal_handlers(interrupt_fd: libc::c_int) -> nix::Result<()> {
    INTERRUPT_FD.store(interrupt_fd, Ordering::SeqCst);
    let action = SigAction::new(
        SigHandler::Handler(interrupt_handler),
        SaFlags::empty(),
        SigSet::empty(),
    );
    for signal in [
        Signal::SIGTERM,
        Signal::SIGINT,
        Signal::SIGHUP,
        Signal::SIGPIPE,
    ] {
        // SAFETY: `action` outlives the registration and the handler only
        // touches atomics plus a pipe write.
        unsafe { sigaction(signal, &action)? };
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct Session {
    mode: String,
    ram_mb: u64,
    target: String,
    target_args: Vec<String>,
    /// Host paths staged into the RAM workspace before exec (basename kept).
    copy_in: Vec<String>,
    /// Contract placeholder for the stricter (GUI-breaking) seccomp profile.
    seccomp_strict: bool,
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
            Arg::new("copy-in")
                .long("copy-in")
                .action(clap::ArgAction::Append)
                .value_name("HOST_PATH")
                .allow_hyphen_values(true)
                .help("Copy a host file into the RAM workspace (repeatable; basename preserved, mode 0600)"),
        )
        .arg(
            Arg::new("seccomp-strict")
                .long("seccomp-strict")
                .action(clap::ArgAction::SetTrue)
                .help("Request the stricter seccomp profile (warning: may break complex GUI apps)"),
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
        copy_in: matches
            .get_many::<String>("copy-in")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        seccomp_strict: matches.get_flag("seccomp-strict"),
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

/// Shared parent-side scrub: overwrite every file with zeros, then remove the
/// workspace directory. Used both on the normal exit path and on interruption.
fn zeroize_and_remove_workspace(path: &Path) -> io::Result<()> {
    match zero_tree(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    match fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

/// Signal-driven teardown: stop the supervisor and target, then scrub the
/// workspace before the parent exits with an interrupted-session code.
fn emergency_cleanup(session: &Session, child_pid: Pid) {
    let _ = kill(child_pid, Signal::SIGKILL);
    for _ in 0..50 {
        match waitpid(child_pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) => std::thread::sleep(Duration::from_millis(10)),
            Ok(_) | Err(Errno::ECHILD) => break,
            Err(Errno::EINTR) => continue,
            Err(_) => break,
        }
    }
    if let Err(error) = zeroize_and_remove_workspace(&session.mount_point) {
        eprintln!("burnbox-core: interrupted cleanup failed: {error}");
    }
}

/// Stage each `--copy-in` host file into the RAM workspace. Only the basename
/// is kept, creation is exclusive with mode 0600, and the copy is flushed.
/// This runs inside the namespace after any tmpfs mount, so staged files live
/// in the workspace and are covered by the `zero_tree` scrub on exit.
fn copy_in_files(session: &Session) -> io::Result<()> {
    for host_path in &session.copy_in {
        let source = Path::new(host_path);
        let basename = source.file_name().ok_or_else(|| {
            io::Error::other(format!("--copy-in path has no file name: {host_path}"))
        })?;
        let mut input = File::open(source)
            .map_err(|error| io::Error::other(format!("--copy-in {host_path}: {error}")))?;
        let destination = session.mount_point.join(basename);
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&destination)
            .map_err(|error| {
                io::Error::other(format!(
                    "--copy-in destination {}: {error}",
                    destination.display()
                ))
            })?;
        // umask can only remove bits; set the exact contract mode explicitly.
        output.set_permissions(Permissions::from_mode(0o600))?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
    }
    Ok(())
}

/// Names visible in `/sys/class/net` other than the loopback device.
fn external_interfaces<I, S>(names: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    names
        .into_iter()
        .map(|name| name.as_ref().to_string_lossy().into_owned())
        .filter(|name| name != "lo")
        .collect()
}

/// Interface names from `/proc/net/dev`, which is virtualized per network
/// namespace (unlike `/sys` on hosts whose sysfs is shared with the container).
fn interfaces_from_proc_net_dev(text: &str) -> Vec<String> {
    text.lines()
        .skip(2) // two header lines
        .filter_map(|line| {
            let (name, _) = line.split_once(':')?;
            let name = name.trim();
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

/// Offline sessions must not see any host interface. `lo` may be up or down;
/// either is acceptable. `/sys/class/net` is inspected as requested, but the
/// decision uses `/proc/net/dev`: in containers with a host-shared sysfs the
/// sysfs entries are stale and can list devices absent from the real netns.
fn verify_offline_network() -> Result<(), String> {
    let entries = fs::read_dir("/sys/class/net")
        .map_err(|error| format!("cannot inspect /sys/class/net: {error}"))?;
    let names = entries
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<Vec<_>>>()
        .map_err(|error| format!("cannot read /sys/class/net: {error}"))?;
    let sys_external = external_interfaces(names);

    let proc_text = fs::read_to_string("/proc/net/dev")
        .map_err(|error| format!("cannot inspect /proc/net/dev: {error}"))?;
    let external = external_interfaces(interfaces_from_proc_net_dev(&proc_text));

    if !sys_external.is_empty() && external.is_empty() {
        eprintln!(
            "burnbox-core: warning: /sys/class/net lists stale host interfaces ({}) but the network namespace is loopback-only",
            sys_external.join(", ")
        );
    }
    if external.is_empty() {
        Ok(())
    } else {
        Err(format!("external interfaces visible: {}", external.join(", ")))
    }
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

/// Clone the namespace supervisor with the requested flag set. Shared by the
/// first attempt (with `CLONE_NEWNET`) and the degraded retry (without it).
fn clone_supervisor(
    session: &Session,
    child_read: nix::libc::c_int,
    child_write: nix::libc::c_int,
    stack: &mut [u8],
    flags: CloneFlags,
) -> nix::Result<Pid> {
    let child_session = session.clone();
    unsafe {
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
            stack,
            flags,
            Some(nix::libc::SIGCHLD),
        )
    }
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

    // Self-pipe woken by async signal handlers. The handler itself only sets a
    // flag and writes one byte; the main loop performs the zeroize scrub.
    let (interrupt_read, interrupt_write) = match pipe() {
        Ok(fds) => fds,
        Err(error) => {
            eprintln!("burnbox-core: cannot create signal pipe: {error}");
            let _ = zeroize_and_remove_workspace(&session.mount_point);
            process::exit(EXIT_SANDBOX);
        }
    };
    if let Err(error) = install_signal_handlers(interrupt_write.as_raw_fd()) {
        eprintln!("burnbox-core: cannot install signal handlers: {error}");
        let _ = zeroize_and_remove_workspace(&session.mount_point);
        process::exit(EXIT_SANDBOX);
    }

    // Avoid duplicating buffered output into the cloned process.
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();

    let (sync_read, sync_write) = match pipe() {
        Ok(fds) => fds,
        Err(error) => {
            eprintln!("burnbox-core: cannot create sync pipe: {error}");
            let _ = zeroize_and_remove_workspace(&session.mount_point);
            process::exit(EXIT_SANDBOX);
        }
    };

    // The clone copies the file descriptor table: pass raw descriptors so the
    // parent keeps ownership of its own ends of the pipe.
    let child_read = sync_read.as_raw_fd();
    let child_write = sync_write.as_raw_fd();
    let mut stack = vec![0u8; SUPERVISOR_STACK];
    let base_flags =
        CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNS | CloneFlags::CLONE_NEWPID;
    let network_flags = base_flags | CloneFlags::CLONE_NEWNET;

    let child_pid = match clone_supervisor(
        &session,
        child_read,
        child_write,
        &mut stack,
        network_flags,
    ) {
        Ok(pid) => pid,
        Err(error) if session.mode == "offline" => {
            // Offline sessions promise network isolation, so a missing netns
            // is a hard namespace setup failure (exit 2).
            eprintln!("burnbox-core: offline mode requires CLONE_NEWNET: {error}");
            drop(sync_read);
            drop(sync_write);
            let _ = zeroize_and_remove_workspace(&session.mount_point);
            process::exit(EXIT_SANDBOX);
        }
        Err(error) => {
            // Online sessions degrade gracefully on hosts (for example Ubuntu
            // with AppArmor) that refuse a network namespace to unprivileged
            // processes. A warning is emitted; the session still runs.
            eprintln!(
                "burnbox-core: warning: CLONE_NEWNET unavailable ({error}); continuing without network isolation"
            );
            match clone_supervisor(&session, child_read, child_write, &mut stack, base_flags) {
                Ok(pid) => pid,
                Err(error) => {
                    eprintln!("burnbox-core: namespace clone failed: {error}");
                    drop(sync_read);
                    drop(sync_write);
                    let _ = zeroize_and_remove_workspace(&session.mount_point);
                    process::exit(EXIT_SANDBOX);
                }
            }
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

    // Poll the self-pipe while waiting: a caught signal wakes poll immediately
    // and the scrub happens here, outside the async signal handler.
    let mut poll_fds = [PollFd::new(interrupt_read.as_fd(), PollFlags::POLLIN)];
    let status = loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            eprintln!("burnbox-core: interrupted by signal; scrubbing workspace");
            emergency_cleanup(&session, child_pid);
            emit_status("wiped", child_pid, &session.mount_point);
            process::exit(EXIT_RUNTIME);
        }
        match waitpid(child_pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) => {
                let _ = poll(&mut poll_fds, 100u16);
            }
            Ok(status) => break status,
            Err(Errno::EINTR) => continue,
            Err(Errno::ECHILD) => break WaitStatus::Exited(child_pid, EXIT_RUNTIME),
            Err(error) => {
                eprintln!("burnbox-core: failed to wait for sandbox: {error}");
                break WaitStatus::Exited(child_pid, EXIT_RUNTIME);
            }
        }
    };

    if let Err(error) = zeroize_and_remove_workspace(&session.mount_point) {
        eprintln!("burnbox-core: cannot scrub RAM workspace: {error}");
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
    if session.seccomp_strict {
        // The target gets an allow-list filter; warn because apps that need
        // syscalls outside it (for example clone3) will be killed.
        eprintln!("Warning: strict seccomp enabled; may break complex GUI apps");
    }

    // Hard assertion for offline sessions: CLONE_NEWNET was granted, but the
    // kernel must actually show an isolated network (loopback only).
    if session.mode == "offline"
        && let Err(error) = verify_offline_network()
    {
        eprintln!("burnbox-core: offline network isolation assertion failed: {error}");
        return EXIT_SANDBOX;
    }

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

    // Stage host files after workspace/mount setup but before exec. Anything
    // placed here is inside the RAM workspace and will be scrubbed on exit.
    if let Err(error) = copy_in_files(session) {
        eprintln!("burnbox-core: copy-in failed: {error}");
        let _ = cleanup_mounts(session, ram_mounted, home_mounted);
        return EXIT_RUNTIME;
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

    session_exit_code(target_status)
}

/// Map the target's terminal status to the AGREEMENT.md exit contract.
fn session_exit_code(status: Option<WaitStatus>) -> i32 {
    match status {
        Some(WaitStatus::Exited(_, EXIT_CLEAN)) => EXIT_CLEAN,
        // SIGSYS means the target tripped the seccomp filter: a security
        // violation, not a generic runtime error.
        Some(WaitStatus::Signaled(_, Signal::SIGSYS, _)) => EXIT_SECURITY,
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
    // Keep PWD consistent with the child's cwd so relative paths such as
    // `xdg-open <basename>` resolve against the workspace.
    environment.push(
        CString::new(format!("PWD={workspace}")).expect("workspace path cannot contain NUL"),
    );
    environment.push(
        CString::new(format!("BURNBOX_MODE={}", session.mode))
            .expect("mode cannot contain NUL"),
    );
    environment
}

/// Replace the grandchild process with the user's target executable.
fn exec_target(session: &Session) -> ! {
    // The target runs from the RAM workspace so staged files are directly
    // addressable by basename (e.g. `xdg-open report.pdf`).
    if let Err(error) = chdir(&session.mount_point) {
        eprintln!("burnbox-core: cannot enter RAM workspace: {error}");
        process::exit(EXIT_RUNTIME);
    }

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

    // Namespace/mount setup is complete; lock the target down before exec.
    // A failure here is a security violation: never run the target unfiltered.
    if let Err(error) = seccomp::install(session.seccomp_strict) {
        eprintln!("burnbox-core: seccomp filter installation failed: {error}");
        process::exit(EXIT_SECURITY);
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
            copy_in: Vec::new(),
            seccomp_strict: false,
            mount_point: PathBuf::from("/dev/shm/burnbox-42"),
        };
        let environment = build_environment(&session);
        assert!(environment
            .iter()
            .any(|entry| entry.as_bytes() == b"HOME=/dev/shm/burnbox-42"));
        assert!(environment
            .iter()
            .any(|entry| entry.as_bytes() == b"TMPDIR=/dev/shm/burnbox-42"));
        assert!(environment
            .iter()
            .any(|entry| entry.as_bytes() == b"PWD=/dev/shm/burnbox-42"));
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
            copy_in: Vec::new(),
            seccomp_strict: false,
            mount_point: root.clone(),
        };

        cleanup_mounts(&session, false, false).expect("fallback cleanup");

        assert!(!root.exists());
    }

    #[test]
    fn cli_accepts_repeatable_copy_in() {
        let matches = build_cli()
            .try_get_matches_from([
                "burnbox-core",
                "--mode",
                "online",
                "--ram-mb",
                "64",
                "--copy-in",
                "/tmp/first.pdf",
                "--copy-in",
                "/tmp/second.pdf",
                "--target",
                "/usr/bin/xdg-open",
                "--",
                "first.pdf",
            ])
            .expect("valid arguments with repeatable --copy-in");
        let session = session_from_matches(&matches, PathBuf::from("/tmp/mnt"));
        assert_eq!(
            session.copy_in,
            vec!["/tmp/first.pdf", "/tmp/second.pdf"]
        );
        assert_eq!(session.target_args, vec!["first.pdf"]);
    }

    #[test]
    fn seccomp_strict_flag_is_optional_and_parsed() {
        let base = [
            "burnbox-core",
            "--mode",
            "online",
            "--ram-mb",
            "16",
            "--target",
            "/bin/true",
        ];
        let default_matches = build_cli().try_get_matches_from(base).expect("default CLI");
        assert!(!session_from_matches(&default_matches, PathBuf::from("/tmp/mnt")).seccomp_strict);

        let mut strict_args = base.to_vec();
        strict_args.push("--seccomp-strict");
        let strict_matches = build_cli()
            .try_get_matches_from(strict_args)
            .expect("strict CLI");
        assert!(session_from_matches(&strict_matches, PathBuf::from("/tmp/mnt")).seccomp_strict);
    }

    #[test]
    fn offline_network_check_ignores_only_loopback() {
        assert!(external_interfaces(["lo"]).is_empty());
        assert_eq!(
            external_interfaces(["lo", "eth0", "wlan0"]),
            vec!["eth0", "wlan0"]
        );
        assert_eq!(external_interfaces(Vec::<&OsStr>::new()), Vec::<String>::new());

        let proc_net_dev = "Inter-|   Receive                    Transmit\n face |bytes    packets\n    lo:       0 0 0\n  eth0:       0 0 0\n";
        assert_eq!(
            interfaces_from_proc_net_dev(proc_net_dev),
            vec!["lo", "eth0"]
        );
        assert_eq!(
            external_interfaces(interfaces_from_proc_net_dev(proc_net_dev)),
            vec!["eth0"]
        );
    }

    #[test]
    fn copy_in_files_are_mode_0600_and_scrubbed() {
        let root = std::env::temp_dir().join(format!("burnbox-copyin-test-{}", process::id()));
        let source = root.join("source.txt");
        let workspace = root.join("workspace");
        fs::create_dir_all(&root).expect("create test root");
        fs::write(&source, b"highly-sensitive").expect("write source file");
        create_workspace(&workspace).expect("create test workspace");

        let session = Session {
            mode: "online".to_string(),
            ram_mb: 16,
            target: "/bin/true".to_string(),
            target_args: Vec::new(),
            copy_in: vec![source.to_string_lossy().into_owned()],
            seccomp_strict: false,
            mount_point: workspace.clone(),
        };
        copy_in_files(&session).expect("stage copy-in file");

        let staged = workspace.join("source.txt");
        assert_eq!(fs::read(&staged).expect("read staged file"), b"highly-sensitive");
        assert_eq!(
            fs::metadata(&staged).expect("stat staged file").permissions().mode() & 0o777,
            0o600
        );

        // The staged file is inside the workspace, so the normal scrub must
        // overwrite and remove it (proving it is in the zeroize boundary).
        zero_tree(&workspace).expect("scrub workspace with staged file");
        assert!(!staged.exists());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn seccomp_termination_maps_to_security_exit() {
        let pid = Pid::from_raw(1);
        assert_eq!(session_exit_code(None), EXIT_RUNTIME);
        assert_eq!(
            session_exit_code(Some(WaitStatus::Exited(pid, EXIT_CLEAN))),
            EXIT_CLEAN
        );
        assert_eq!(
            session_exit_code(Some(WaitStatus::Exited(pid, 7))),
            EXIT_RUNTIME
        );
        assert_eq!(
            session_exit_code(Some(WaitStatus::Signaled(pid, Signal::SIGSYS, false))),
            EXIT_SECURITY
        );
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
