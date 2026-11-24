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

## Rootless online networking (`slirp4netns`)

`online` sessions try to give the isolated network namespace real TCP/IP
without any host privileges. After the uid/gid maps are installed, the parent
searches `PATH` for `slirp4netns` (the optional `BURNBOX_SLIRP4NETNS` environment
variable can point at a binary installed off-PATH) and, when found, launches it
against the supervisor's PID:

```
slirp4netns --configure --ready-fd=1 <supervisor-pid> burnbox0
```

Only the host-side parent spawns the helper — the target never gains the
ability to create host interfaces. `--configure` brings up `burnbox0` as
`10.0.2.100/24` with the slirp gateway/DNS at `10.0.2.2`/`10.0.2.3`. The parent
waits for the one-byte readiness notification on the helper's stdout **before**
releasing the supervisor to `execve` the target, so no process ever sees a
half-configured interface. The helper is killed and reaped on every exit path,
including the signal-driven scrub.

If the binary is missing, the session still runs and writes exactly this line
to stderr (never stdout):

```
Warning: slirp4netns not found; network isolation is strict loopback only. Install slirp4netns for live URL fetching.
```

`offline` sessions never invoke the helper: the external interface would
violate their loopback-only assertion. `slirp4netns` is a runtime dependency,
not a build dependency; CI hosts without it keep the previous behavior.

`tests/slirp_network.rs` covers the missing-binary warning, the exact spawn
arguments using an executable stub, and an end-to-end tap-configuration check
that self-skips when no real `slirp4netns` is available.

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
switches to an **allow-list** of syscalls needed by real viewers. Any syscall
outside that list — including every entry of the deny-list above — is killed,
yielding exit code `3`. The flag does not change the two JSON stdout lines.

The allow-list was validated against a real headless Firefox run on a tiny
local file and PDF (Evince). Beyond ordinary file/memory/signal/socket calls,
it includes viewer-specific entries, each added only after strace evidence:

- `clone`/`clone3`/`fork`/`vfork`/`waitid` — shell launchers and GUI threads.
- `tgkill`, `fadvise64` (arm: `arm_fadvise64_64`) — glibc thread signals and
  locale file reads.
- `pidfd_open`/`pidfd_send_signal` — GLib child-process monitoring.
- `seccomp` — snap's kernel feature probe. Filters are additive, so a viewer
  probing seccomp cannot weaken BurnBox's installed filter.
- `getresuid`/`getresgid`, `capget` — read-only snap confinement checks.
- `restart_syscall` — resume of a `SA_RESTART` futex wait after a signal.
- `readahead` — ELF-loader prefetch of Firefox shared libraries.
- `unshare` (PID only), `setsid`, `setpgid` — Firefox's own content-sandbox and
  process detachment. `setns`, `chroot` and `pivot_root` stay killed.
- `socketpair`, `sendmmsg`, `recvmmsg` — Firefox IPC.
- `symlink`/`symlinkat`, `link`/`linkat`, `utimensat`, `fallocate`, `sendfile`,
  `quotactl` — profile setup, font cache and GLib quota probes.
- `memfd_create` — anonymous in-memory files for shared-memory IPC.
- `getpriority`/`setpriority`, `ioprio_get`/`ioprio_set`, `sched_*` queries —
  thread-pool and background-I/O priority management.

Dangerous primitives (`ptrace`, `mount`, `umount2`, `bpf`, `seccomp` filter
updates that would drop privileges, etc.) remain blocked. A BPF-evaluator unit
test asserts both modes still kill `ptrace`, `mount` and a mismatched
architecture. `tests/viewer_strict.rs` runs a strict-mode open of a tiny file
through any available native Firefox binary; it self-skips when none is
installed so CI is never blocked, and asserts exit `0`, exactly two JSON lines,
and no `/dev/shm/burnbox-*` leak.

SysV IPC (`shmget`/`shmat`/`msgget`/...) is intentionally **not** in the
allow-list: BurnBox does not unshare the host IPC namespace, so permitting those
calls would let the target exchange shared memory with unrelated host
processes. Adding them safely would first require `CLONE_NEWIPC`.

### Diagnosing a missing allow-list entry

Setting `BURNBOX_SECCOMP_DIAG=1` replaces the `KILL_PROCESS` action with
`SECCOMP_RET_ERRNO | ENOSYS` and prints a warning to stderr. Under `strace`, a
missing entry is named instead of appearing as an unexplained `SIGSYS`:

```
BURNBOX_SECCOMP_DIAG=1 strace -f -o /tmp/ff.strace \
  ./bin/burnbox-core --mode online --ram-mb 512 --seccomp-strict \
  --target /path/to/native/firefox -- --headless --screenshot=/tmp/ff.png file:///etc/hostname
grep ENOSYS /tmp/ff.strace
```

This is a diagnostic switch only; it must never be enabled in production. The
option is covered by a BPF-evaluator unit test. Using it on the native Firefox
path showed no ENOSYS from BurnBox's filter: `clone3` and `sched_getaffinity`
ENOSYS lines came from Firefox's own nested content-process sandbox, while the
main process' calls succeeded.

Note: on hosts where `/usr/bin/firefox` is a snap wrapper (a `#!/bin/sh` script
that execs setuid `snap-confine`), that wrapper exits `1` with
`cap_dac_override not found in current capabilities` even with **no seccomp
filter at all**. Unprivileged user namespaces drop the file/setuid privileges
snap-confine needs, so no allow-list change can fix it. Use the native ELF
binary (for example `/snap/firefox/current/usr/lib/firefox/firefox`) for strict
viewer runs; the integration test discovers it automatically.

Without `slirp4netns`, `online` sessions keep only loopback networking and an
external `https://` URL stalls on DNS (not a seccomp kill). With the helper
installed, live URL fetching works without host privileges.

## stdout / exit contract

Only two lines are ever printed to stdout (exact key order and spacing):

```
{"status": "started", "pid": <int>, "mount_point": "<path>"}
{"status": "wiped", "pid": <int>, "mount_point": "<path>"}
```

All diagnostics go to stderr. Exit codes: `0` clean, `1` runtime error,
`2` namespace/setup failure, `3` security violation or seccomp failure.
