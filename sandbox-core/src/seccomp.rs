//! Minimal seccomp-BPF policy applied to the target immediately before exec.
//!
//! The filter is deliberately tiny and dependency-free: it uses raw
//! `seccomp(2)` through `libc`, so no `libseccomp` development package or sudo
//! is required. Setting `PR_SET_NO_NEW_PRIVS` first allows an unprivileged
//! process to install the filter. Any syscall not listed below is allowed;
//! blocked syscalls (and any unexpected architecture) terminate the process
//! with `SECCOMP_RET_KILL_PROCESS`, which the supervisor maps to exit code 3.

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

/// Syscalls denied to the target with `SECCOMP_RET_KILL_PROCESS`.
///
/// The list covers kernel-module loading and kexec, ptrace/introspection,
/// cross-process memory reads/writes, file-handle and fanotify access,
/// BPF/perf/userfaultfd primitives, and mount operations (which reinforce the
/// AppArmor boundary). These constants exist on every architecture supported
/// by this crate.
fn blocked_syscalls() -> &'static [libc::c_long] {
    &[
        // Kernel manipulation and escape primitives.
        libc::SYS_ptrace,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_kexec_load,
        // Mount operations (AppArmor backup).
        libc::SYS_mount,
        libc::SYS_umount2,
        // Introspection / kernel subsystems.
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

fn instruction(code: u16, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

/// Build the classic-BPF program:
///   load arch -> reject wrong arch -> load nr -> reject blocked nrs -> allow.
fn build_filter() -> Vec<libc::sock_filter> {
    let blocked = blocked_syscalls();
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

/// Install the filter in the current process. Must be called after namespace
/// setup and immediately before `execve` so only the target is restricted.
pub fn install() -> Result<(), String> {
    let mut program = build_filter();
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

    #[test]
    fn filter_contains_all_required_blocks() {
        let program = build_filter();
        let blocked = blocked_syscalls();
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
}
