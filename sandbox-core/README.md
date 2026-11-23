# burnbox-core

Unprivileged sandbox and RAM-wipe engine for BurnBox. No `sudo` is required.

## Namespace strategy

Each session is created with `clone(CLONE_NEWUSER | CLONE_NEWNS | CLONE_NEWPID)`.
A network namespace (`CLONE_NEWNET`) is requested in the same call:

- If the host grants it, the target runs with no visible host network interfaces.
- If the kernel or host LSM refuses it (common on Ubuntu with AppArmor), the
  behavior depends on `--mode`:
  - `online`: a warning is written to **stderr** and the session is retried
    without `CLONE_NEWNET`. The target still runs; no crash.
  - `offline`: network isolation is mandatory, so the session fails as a
    namespace setup error (exit code `2`) with no target execution.

The parent process (the namespace creator) writes `uid_map`/`gid_map` and
signals the supervisor through a pipe before any mount happens.

## RAM workspace and AppArmor fallback

The workspace path is `/dev/shm/burnbox-<pid>`, which is kernel `tmpfs`.
`burnbox-core` first tries to mount a fresh, size-limited `tmpfs` there and to
hide the host `/home` behind an empty `tmpfs`. Ubuntu's `unprivileged_userns`
AppArmor profile denies `mount(2)` inside user namespaces with `EACCES`; in that
case the workspace directory itself is used directly (mode `0700`), `HOME` and
`TMPDIR` are redirected to it, and the `/home` overlay is skipped with a
warning. On exit every file is overwritten with a `zeroize`-scrubbed zero buffer
and the directory is removed (or the tmpfs is unmounted).

## Seccomp-BPF

Immediately before `execve`, the target process installs a small classic-BPF
filter via raw `seccomp(2)` (no `libseccomp` system dependency). `PR_SET_NO_NEW_PRIVS`
is set first so unprivileged installation is permitted. The filter kills the
process on:

- `ptrace`
- `init_module`, `finit_module`
- `kexec_load`
- `mount` (reinforces the AppArmor boundary)

Any other syscall is allowed. An unexpected architecture also terminates the
process. Failure to install the filter returns exit code `3` and the target is
never executed; a target killed by the filter (`SIGSYS`) also makes the session
exit with code `3`.

## stdout / exit contract

Only two lines are ever printed to stdout (exact key order and spacing):

```
{"status": "started", "pid": <int>, "mount_point": "<path>"}
{"status": "wiped", "pid": <int>, "mount_point": "<path>"}
```

All diagnostics go to stderr. Exit codes: `0` clean, `1` runtime error,
`2` namespace/setup failure, `3` security violation or seccomp failure.
