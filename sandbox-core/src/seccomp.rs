//! Seccomp-BPF policy applied to the target immediately before exec.
//!
//! The filter is dependency-free: it uses raw `seccomp(2)` through `libc`, so
//! no `libseccomp` package or sudo is required. Setting `PR_SET_NO_NEW_PRIVS`
//! first allows an unprivileged process to install it.
//!
//! Two policies are available:
//! - **default**: a deny-list of introspection/escape syscalls; everything
//!   else is allowed.
//! - **strict** (`--seccomp-strict`): an allow-list of syscalls needed by
//!   typical viewers; everything else is killed.
//!
//! Both use `SECCOMP_RET_KILL_PROCESS`, which the supervisor maps to exit 3.

use nix::errno::Errno;
use nix::libc;

// Classic BPF instruction encodings used by seccomp filters.
const BPF_LD_W_ABS: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS
const BPF_JEQ_K: u16 = 0x15; // BPF_JMP | BPF_JEQ | BPF_K
const BPF_RET_K: u16 = 0x06; // BPF_RET | BPF_K

// seccomp_data offsets (see seccomp(2)).
const SECCOMP_DATA_NR_OFFSET: u32 = 0;
const SECCOMP_DATA_ARCH_OFFSET: u32 = 4;

const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;

// AUDIT_ARCH_* values from linux/audit.h.
#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH: u32 = 0xc000_003e;
#[cfg(target_arch = "aarch64")]
const AUDIT_ARCH: u32 = 0xc000_00b7;
#[cfg(target_arch = "x86")]
const AUDIT_ARCH: u32 = 0x4000_0003;
#[cfg(target_arch = "arm")]
const AUDIT_ARCH: u32 = 0x4000_0028;
#[cfg(target_arch = "riscv64")]
const AUDIT_ARCH: u32 = 0xc000_00f3;
#[cfg(not(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "x86",
    target_arch = "arm",
    target_arch = "riscv64"
)))]
compile_error!("burnbox-core seccomp filter does not know this architecture");

/// Syscalls denied in the default policy with `SECCOMP_RET_KILL_PROCESS`.
///
/// Covers kernel-module loading and kexec, ptrace/introspection, cross-process
/// memory access, file-handle and fanotify access, BPF/perf/userfaultfd
/// primitives, and mount operations (AppArmor backup).
fn denied_syscalls() -> &'static [libc::c_long] {
    &[
        libc::SYS_ptrace,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_kexec_load,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_userfaultfd,
        libc::SYS_kcmp,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_name_to_handle_at,
        libc::SYS_open_by_handle_at,
        libc::SYS_fanotify_init,
    ]
}

/// Syscalls permitted in strict mode. This is a deliberately generous list of
/// the normal file/memory/signal/scheduling/socket calls required to load and
/// run typical GUI viewers. Anything not here is killed, including every
/// entry of `denied_syscalls()`.
fn allowed_syscalls() -> Vec<libc::c_long> {
    // Syscall numbers that exist under the same libc name on every supported
    // architecture.
    let mut allowed = vec![
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_close,
        libc::SYS_fstat,
        libc::SYS_statfs,
        libc::SYS_fstatfs,
        libc::SYS_lseek,
        libc::SYS_mprotect,
        libc::SYS_munmap,
        libc::SYS_brk,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        // Signal handlers installed with SA_RESTART resume an interrupted
        // wait through this pseudo-syscall; without it viewer threads die on
        // the first delivered signal.
        libc::SYS_restart_syscall,
        libc::SYS_sigaltstack,
        libc::SYS_ioctl,
        libc::SYS_pread64,
        // Firefox's ELF loader prefetches shared libraries with readahead.
        libc::SYS_readahead,
        libc::SYS_pwrite64,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_sched_yield,
        libc::SYS_mremap,
        libc::SYS_msync,
        libc::SYS_mincore,
        libc::SYS_madvise,
        libc::SYS_dup,
        libc::SYS_getpid,
        libc::SYS_getppid,
        libc::SYS_getuid,
        libc::SYS_geteuid,
        libc::SYS_getgid,
        libc::SYS_getegid,
        // Snap's setuid helpers read the full real/effective/saved ID set.
        libc::SYS_getresuid,
        libc::SYS_getresgid,
        // Snap checks its confinement with a read-only capability query.
        libc::SYS_capget,
        libc::SYS_gettid,
        libc::SYS_execve,
        libc::SYS_exit,
        libc::SYS_exit_group,
        libc::SYS_wait4,
        // Process/thread creation is required by shell-based viewer launchers
        // (`/usr/bin/firefox` is a script) and by browser/GUI toolkits that
        // spawn threads. `ptrace`, `mount` and the other escape primitives stay
        // on the denied side; CLONE_NEW* flags are not separately filtered by
        // this classic-BPF program.
        libc::SYS_clone,
        libc::SYS_clone3,
        libc::SYS_waitid,
        libc::SYS_kill,
        // glibc uses tgkill for pthread signals and fadvise64 to hint sequential
        // locale/library reads; both appear in real viewer startup traces.
        libc::SYS_tgkill,
        // Firefox's content sandbox calls unshare(CLONE_NEWPID). Namespace
        // creation cannot restore privileges or escape the mount restrictions
        // already enforced, so it is allowed; setns/chroot/pivot_root stay
        // killed.
        libc::SYS_unshare,
        libc::SYS_setsid,
        libc::SYS_setpgid,
        // GLib/GIO watches child processes with pidfds; signalling through a
        // pidfd is the corresponding teardown call.
        libc::SYS_pidfd_open,
        libc::SYS_pidfd_send_signal,
        libc::SYS_uname,
        // Anonymous in-memory files back browser shared-memory IPC.
        libc::SYS_memfd_create,
        libc::SYS_fcntl,
        libc::SYS_flock,
        libc::SYS_fsync,
        libc::SYS_fdatasync,
        libc::SYS_truncate,
        libc::SYS_ftruncate,
        // Firefox preallocates its profile database and copies cached data
        // with sendfile; GLib probes filesystem quotas on tmpfs.
        libc::SYS_fallocate,
        libc::SYS_sendfile,
        libc::SYS_quotactl,
        libc::SYS_getdents64,
        libc::SYS_getcwd,
        libc::SYS_chdir,
        libc::SYS_fchmod,
        libc::SYS_fchown,
        libc::SYS_umask,
        libc::SYS_gettimeofday,
        libc::SYS_getrusage,
        libc::SYS_sysinfo,
        libc::SYS_times,
        libc::SYS_getrandom,
        libc::SYS_clock_gettime,
        libc::SYS_clock_getres,
        libc::SYS_clock_nanosleep,
        libc::SYS_nanosleep,
        libc::SYS_sched_getscheduler,
        libc::SYS_sched_getaffinity,
        libc::SYS_sched_setaffinity,
        libc::SYS_sched_getparam,
        libc::SYS_sched_get_priority_max,
        libc::SYS_sched_get_priority_min,
        libc::SYS_timerfd_create,
        libc::SYS_timerfd_settime,
        // Firefox's thread pool queries its scheduling priority at startup.
        libc::SYS_getpriority,
        libc::SYS_setpriority,
        // Firefox's background I/O thread queries/sets its I/O priority.
        libc::SYS_ioprio_get,
        libc::SYS_ioprio_set,
        libc::SYS_set_tid_address,
        libc::SYS_set_robust_list,
        libc::SYS_futex,
        libc::SYS_epoll_create1,
        libc::SYS_epoll_ctl,
        libc::SYS_epoll_pwait,
        libc::SYS_ppoll,
        libc::SYS_eventfd2,
        libc::SYS_inotify_init1,
        libc::SYS_inotify_add_watch,
        libc::SYS_prctl,
        // Snap-based launchers probe seccomp(2) (SECCOMP_SET_MODE_STRICT) to
        // fingerprint the kernel. Filters are additive and can only restrict
        // further, so allowing this cannot weaken the BurnBox filter.
        libc::SYS_seccomp,
        libc::SYS_rseq,
        libc::SYS_membarrier,
        libc::SYS_prlimit64,
        libc::SYS_dup3,
        libc::SYS_statx,
        libc::SYS_openat,
        libc::SYS_mkdirat,
        libc::SYS_unlinkat,
        libc::SYS_faccessat,
        libc::SYS_faccessat2,
        libc::SYS_readlinkat,
        libc::SYS_fchmodat,
        libc::SYS_fchownat,
        // Firefox creates/hardlinks profile and font-cache files through the
        // *at variants and stamps them with utimensat.
        libc::SYS_linkat,
        libc::SYS_symlinkat,
        libc::SYS_utimensat,
        libc::SYS_pipe2,
        libc::SYS_accept4,
        libc::SYS_socket,
        // Firefox IPC uses socketpair and batched datagram I/O.
        libc::SYS_socketpair,
        libc::SYS_sendmmsg,
        libc::SYS_recvmmsg,
        libc::SYS_connect,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_sendto,
        libc::SYS_recvfrom,
        libc::SYS_sendmsg,
        libc::SYS_recvmsg,
        libc::SYS_shutdown,
        libc::SYS_getsockname,
        libc::SYS_getpeername,
        libc::SYS_setsockopt,
        libc::SYS_getsockopt,
    ];

    // Legacy 32-bit names live only on x86/arm; 64-bit arches use the *at
    // variants and `newfstatat` instead.
    #[cfg(target_arch = "x86_64")]
    allowed.extend_from_slice(&[
        libc::SYS_fork,
        libc::SYS_vfork,
        libc::SYS_mmap,
        libc::SYS_arch_prctl,
        libc::SYS_open,
        libc::SYS_stat,
        libc::SYS_lstat,
        libc::SYS_newfstatat,
        libc::SYS_poll,
        libc::SYS_select,
        libc::SYS_access,
        libc::SYS_pipe,
        libc::SYS_dup2,
        libc::SYS_getdents,
        libc::SYS_rename,
        libc::SYS_mkdir,
        libc::SYS_rmdir,
        libc::SYS_unlink,
        libc::SYS_readlink,
        libc::SYS_symlink,
        libc::SYS_link,
        libc::SYS_chmod,
        libc::SYS_chown,
        libc::SYS_getrlimit,
        libc::SYS_setrlimit,
        libc::SYS_epoll_wait,
        libc::SYS_accept,
    ]);
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    allowed.extend_from_slice(&[
        libc::SYS_mmap,
        libc::SYS_newfstatat,
        libc::SYS_renameat2,
    ]);
    #[cfg(target_arch = "x86")]
    allowed.extend_from_slice(&[
        libc::SYS_fork,
        libc::SYS_vfork,
        libc::SYS_fadvise64_64,
        libc::SYS_mmap,
        libc::SYS_mmap2,
        libc::SYS_open,
        libc::SYS_stat,
        libc::SYS_lstat,
        libc::SYS_poll,
        libc::SYS_select,
        libc::SYS_access,
        libc::SYS_pipe,
        libc::SYS_dup2,
        libc::SYS_getdents,
        libc::SYS_rename,
        libc::SYS_mkdir,
        libc::SYS_rmdir,
        libc::SYS_unlink,
        libc::SYS_readlink,
        libc::SYS_symlink,
        libc::SYS_link,
        libc::SYS_chmod,
        libc::SYS_chown,
        libc::SYS_getrlimit,
        libc::SYS_setrlimit,
        libc::SYS_epoll_wait,
        libc::SYS_accept,
    ]);
    #[cfg(target_arch = "arm")]
    allowed.extend_from_slice(&[
        libc::SYS_fork,
        libc::SYS_vfork,
        libc::SYS_mmap2,
        libc::SYS_open,
        libc::SYS_stat,
        libc::SYS_lstat,
        libc::SYS_poll,
        libc::SYS_access,
        libc::SYS_pipe,
        libc::SYS_dup2,
        libc::SYS_getdents,
        libc::SYS_rename,
        libc::SYS_mkdir,
        libc::SYS_rmdir,
        libc::SYS_unlink,
        libc::SYS_readlink,
        libc::SYS_symlink,
        libc::SYS_link,
        libc::SYS_chmod,
        libc::SYS_chown,
        libc::SYS_setrlimit,
        libc::SYS_epoll_wait,
        libc::SYS_accept,
    ]);
    // `fadvise64` is present on every supported arch except 32-bit arm, which
    // exposes the same hint through the arm-specific `arm_fadvise64_64`.
    #[cfg(not(target_arch = "arm"))]
    allowed.push(libc::SYS_fadvise64);
    #[cfg(target_arch = "arm")]
    allowed.push(libc::SYS_arm_fadvise64_64);

    allowed
}

fn instruction(code: u16, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

/// Build the classic-BPF program for the requested policy.
fn build_filter(strict: bool) -> Vec<libc::sock_filter> {
    if strict {
        build_allow_filter()
    } else {
        build_deny_filter()
    }
}

/// Default policy: load arch -> reject wrong arch -> load nr -> reject blocked
/// nrs -> allow.
fn build_deny_filter() -> Vec<libc::sock_filter> {
    let blocked = denied_syscalls();
    // Layout: [0]=ld arch, [1]=jeq arch, [2]=ld nr, [3..3+N]=jeq nr,
    //         [3+N]=allow, [4+N]=kill.
    let kill_index = 4 + blocked.len();
    let mut program = Vec::with_capacity(kill_index + 1);

    program.push(instruction(BPF_LD_W_ABS, 0, 0, SECCOMP_DATA_ARCH_OFFSET));
    program.push(instruction(
        BPF_JEQ_K,
        0,
        (kill_index - 2) as u8,
        AUDIT_ARCH,
    ));
    program.push(instruction(BPF_LD_W_ABS, 0, 0, SECCOMP_DATA_NR_OFFSET));

    for (offset, syscall) in blocked.iter().enumerate() {
        let index = 3 + offset;
        let jt = (kill_index - index - 1) as u8;
        program.push(instruction(BPF_JEQ_K, jt, 0, *syscall as u32));
    }

    program.push(instruction(BPF_RET_K, 0, 0, SECCOMP_RET_ALLOW));
    program.push(instruction(
        BPF_RET_K,
        0,
        0,
        SECCOMP_RET_KILL_PROCESS,
    ));
    program
}

/// Strict policy: load arch -> reject wrong arch -> load nr -> jump to ALLOW
/// on a listed nr, falling through to the KILL instruction otherwise.
fn build_allow_filter() -> Vec<libc::sock_filter> {
    let allowed = allowed_syscalls();
    // Layout: [0]=ld arch, [1]=jeq arch, [2]=ld nr, [3..3+N)=jeq nr,
    //         [3+N]=kill (default fall-through), [4+N]=allow (jump target).
    let kill_index = 3 + allowed.len();
    let allow_index = kill_index + 1;
    let mut program = Vec::with_capacity(allow_index + 1);

    program.push(instruction(BPF_LD_W_ABS, 0, 0, SECCOMP_DATA_ARCH_OFFSET));
    program.push(instruction(
        BPF_JEQ_K,
        0,
        (kill_index - 2) as u8,
        AUDIT_ARCH,
    ));
    program.push(instruction(BPF_LD_W_ABS, 0, 0, SECCOMP_DATA_NR_OFFSET));

    for (offset, syscall) in allowed.iter().enumerate() {
        let index = 3 + offset;
        let jt = (allow_index - index - 1) as u8;
        program.push(instruction(BPF_JEQ_K, jt, 0, *syscall as u32));
    }

    // Fall-through (no listed nr matched) kills; matched syscalls jump to the
    // ALLOW instruction below.
    program.push(instruction(
        BPF_RET_K,
        0,
        0,
        SECCOMP_RET_KILL_PROCESS,
    ));
    program.push(instruction(BPF_RET_K, 0, 0, SECCOMP_RET_ALLOW));
    program
}

/// Install the filter in the current process. Must be called after namespace
/// setup and immediately before `execve` so only the target is restricted.
pub fn install(strict: bool) -> Result<(), String> {
    let mut program = build_filter(strict);
    let mut filter = libc::sock_fprog {
        len: program.len() as libc::c_ushort,
        filter: program.as_mut_ptr(),
    };

    // SAFETY: prctl takes only integer arguments here.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(format!(
            "prctl(PR_SET_NO_NEW_PRIVS): {}",
            Errno::last()
        ));
    }

    // SAFETY: filter points at a live sock_fprog whose `filter` member points
    // at a live sock_filter array; the kernel copies both during the syscall.
    let result = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            0,
            &raw mut filter,
        )
    };
    if result != 0 {
        return Err(format!(
            "seccomp(SECCOMP_SET_MODE_FILTER): {}",
            Errno::last()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny classic-BPF evaluator used to verify the generated program's
    /// decision for a given syscall number without installing it.
    fn evaluate(program: &[libc::sock_filter], nr: libc::c_long, arch: u32) -> u32 {
        let mut accumulator = 0u32;
        let mut pc = 0usize;
        loop {
            let ins = program[pc];
            match ins.code {
                BPF_LD_W_ABS => {
                    accumulator = match ins.k {
                        SECCOMP_DATA_NR_OFFSET => nr as u32,
                        SECCOMP_DATA_ARCH_OFFSET => arch,
                        _ => panic!("unexpected load offset"),
                    };
                    pc += 1;
                }
                BPF_JEQ_K => {
                    let offset = if accumulator == ins.k { ins.jt } else { ins.jf };
                    pc += 1 + usize::from(offset);
                }
                BPF_RET_K => return ins.k,
                _ => panic!("unexpected BPF opcode"),
            }
        }
    }

    #[test]
    fn filters_make_the_expected_decisions() {
        for strict in [false, true] {
            let program = build_filter(strict);
            assert_eq!(
                evaluate(&program, libc::SYS_read, AUDIT_ARCH),
                SECCOMP_RET_ALLOW,
                "read must be allowed in both modes"
            );
            assert_eq!(
                evaluate(&program, libc::SYS_ptrace, AUDIT_ARCH),
                SECCOMP_RET_KILL_PROCESS,
                "ptrace must be killed in both modes"
            );
            assert_eq!(
                evaluate(&program, libc::SYS_mount, AUDIT_ARCH),
                SECCOMP_RET_KILL_PROCESS,
                "mount must be killed in both modes"
            );
            assert_eq!(
                evaluate(&program, libc::SYS_read, AUDIT_ARCH ^ 1),
                SECCOMP_RET_KILL_PROCESS,
                "wrong architecture must be killed"
            );
        }
        assert_eq!(
            evaluate(&build_filter(true), 10_000, AUDIT_ARCH),
            SECCOMP_RET_KILL_PROCESS,
            "strict mode must kill unlisted syscalls"
        );
    }

    #[test]
    fn deny_filter_contains_all_required_blocks() {
        let program = build_filter(false);
        let blocked = denied_syscalls();
        let kill_index = program.len() - 1;

        assert_eq!(program.len(), blocked.len() + 5);
        assert_eq!(program[0].code, BPF_LD_W_ABS);
        assert_eq!(program[0].k, SECCOMP_DATA_ARCH_OFFSET);
        assert_eq!(program[1].code, BPF_JEQ_K);
        assert_eq!(program[1].k, AUDIT_ARCH);
        assert_eq!(usize::from(program[1].jf) + 2, kill_index);
        assert_eq!(program[2].k, SECCOMP_DATA_NR_OFFSET);
        assert_eq!(program[kill_index - 1].k, SECCOMP_RET_ALLOW);
        assert_eq!(program[kill_index].k, SECCOMP_RET_KILL_PROCESS);

        // Explicitly assert every syscall required by the Milestone 5 contract.
        for syscall in [
            libc::SYS_bpf,
            libc::SYS_perf_event_open,
            libc::SYS_userfaultfd,
            libc::SYS_kcmp,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
            libc::SYS_name_to_handle_at,
            libc::SYS_open_by_handle_at,
            libc::SYS_fanotify_init,
            libc::SYS_ptrace,
            libc::SYS_init_module,
            libc::SYS_finit_module,
            libc::SYS_kexec_load,
            libc::SYS_mount,
            libc::SYS_umount2,
        ] {
            assert!(blocked.contains(&syscall), "missing blocked syscall {syscall}");
        }

        for (offset, syscall) in blocked.iter().enumerate() {
            let instruction = program[3 + offset];
            assert_eq!(instruction.code, BPF_JEQ_K);
            assert_eq!(instruction.k, *syscall as u32);
            assert_eq!(usize::from(instruction.jt) + 4 + offset, kill_index);
        }
    }

    #[test]
    fn strict_filter_is_allow_list_with_default_kill() {
        let program = build_filter(true);
        let allowed = allowed_syscalls();
        let kill_index = 3 + allowed.len();
        let allow_index = kill_index + 1;

        assert_eq!(program.len(), allow_index + 1);
        assert_eq!(program[0].code, BPF_LD_W_ABS);
        assert_eq!(program[1].k, AUDIT_ARCH);
        assert_eq!(usize::from(program[1].jf) + 2, kill_index);
        assert_eq!(program[allow_index].k, SECCOMP_RET_ALLOW);
        assert_eq!(program[kill_index].k, SECCOMP_RET_KILL_PROCESS);

        for (offset, syscall) in allowed.iter().enumerate() {
            let instruction = program[3 + offset];
            assert_eq!(instruction.code, BPF_JEQ_K);
            assert_eq!(instruction.k, *syscall as u32);
            assert_eq!(usize::from(instruction.jt) + 4 + offset, allow_index);
        }

        // Viewer basics are permitted, escape primitives are not.
        for syscall in [
            libc::SYS_read,
            libc::SYS_write,
            libc::SYS_openat,
            libc::SYS_futex,
            libc::SYS_rt_sigaction,
            libc::SYS_mprotect,
        ] {
            assert!(allowed.contains(&syscall), "missing allowed syscall {syscall}");
        }
        for syscall in denied_syscalls() {
            assert!(!allowed.contains(syscall), "strict mode allows {syscall}");
        }
    }
}
