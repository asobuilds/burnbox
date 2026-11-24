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
    namespace setup error (exit code `2`) with no target execution. It also
    asserts isolation after clone by reading `/sys/class/net` and the
    netns-authoritative `/proc/net/dev`: only `lo` (up or down) may appear;
    any external interface or an unreadable source aborts setup with exit
    code `2`. On hosts where `/sys` is shared with the container, sysfs can
    list stale host devices; those are warned about on stderr but the
    authoritative netns view decides the assertion.

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

The parent catches `SIGTERM`, `SIGINT`, `SIGHUP` and `SIGPIPE`. Its handler is
async-signal-safe: it only sets an atomic flag and writes one byte to a
self-pipe. The main loop then kills the supervisor, performs the same zeroize +
rmdir scrub, emits the `wiped` line and exits with code `1`. An integration test
(`tests/signal_scrub.rs`) proves a mid-session `SIGTERM` leaves no workspace.

## Staging files with `--copy-in`

`--copy-in <host_path>` may be repeated. After workspace/mount setup but before
`execve`, each file is copied into the workspace root with its basename
preserved and mode `0600` (exclusive creation; duplicate basenames or unreadable
sources fail the session with exit `1`). The target's cwd (and `PWD`) is the
workspace, so `xdg-open <basename>` resolves the staged file. Because staged
files live inside the workspace, the normal exit scrub walks them, overwrites
their contents with zeros, and removes them.

## Seccomp-BPF

Immediately before `execve`, the target process installs a small classic-BPF
filter via raw `seccomp(2)` (no `libseccomp` system dependency). `PR_SET_NO_NEW_PRIVS`
is set first so unprivileged installation is permitted. The filter kills the
process on:

- `ptrace`, `kcmp`, `process_vm_readv`, `process_vm_writev`
- `init_module`, `finit_module`, `kexec_load`
- `bpf`, `perf_event_open`, `userfaultfd`
- `name_to_handle_at`, `open_by_handle_at`, `fanotify_init`
- `mount`, `umount2` (reinforce the AppArmor boundary)

Any other syscall is allowed. An unexpected architecture also terminates the
process. Failure to install the filter returns exit code `3` and the target is
never executed; a target killed by the filter (`SIGSYS`) also makes the session
exit with code `3`.

The optional `--seccomp-strict` flag logs
`Warning: strict seccomp enabled; may break complex GUI apps` to stderr and
switches to an **allow-list** of syscalls needed by typical viewers (file I/O,
memory, signals, scheduling, sockets, etc.). Any syscall outside that list —
including every entry of the deny-list above — is killed, yielding exit code
`3`. The flag does not change the two JSON stdout lines.

## stdout / exit contract

Only two lines are ever printed to stdout (exact key order and spacing):

```
{"status": "started", "pid": <int>, "mount_point": "<path>"}
{"status": "wiped", "pid": <int>, "mount_point": "<path>"}
```

All diagnostics go to stderr. Exit codes: `0` clean, `1` runtime error,
`2` namespace/setup failure, `3` security violation or seccomp failure.
