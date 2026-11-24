# Conflict & Error Resolution Protocol

## 1. Concurrency & Git Rules
- **Atomic Commits:** Each agent must commit working changes to its own directory immediately after passing tests (`git add <allowed_path> && git commit -m "<agent>: <summary>"`).
- **No Force Pushing / No Global Resets:** Never run `git reset --hard` or `git clean -fd` without explicit user approval, as it will destroy the concurrent agent's uncommitted work.
- **Lock Contention:** If an agent needs to edit `AGREEMENT.md` or `RESOLUTION.md` and detects uncommitted changes in `git status`, wait 10 seconds, re-read the file, and append cleanly.

## 2. Error Escalation & Handling
- **Compilation Breakage:** If a change breaks the build in your domain, fix it immediately before touching new features.
- **Cross-Domain Mismatch:** If `burnbox` (Go) fails to parse output from `burnbox-core` (Rust), log the exact stdout/stderr under **Active Incident Log** below rather than editing the other agent's codebase.

## 3. Active Incident Log

### 2026-11-23 — Host AppArmor blocks tmpfs mount in unprivileged user namespaces (Agent A)
- **Symptom:** On this host, `burnbox-core` creates `CLONE_NEWUSER|CLONE_NEWNS|CLONE_NEWPID` successfully and the parent writes `uid_map`/`gid_map` correctly (child sees uid 0 with a full capability set), but `mount("tmpfs", ...)` returns `EACCES`.
- **Root cause:** The kernel automatically confines new user namespaces with the Ubuntu AppArmor profile `unprivileged_userns (enforce)` (`/proc/self/attr/current` in the child), which denies mount operations. Host user `student` has no sudo/AppArmor admin rights, so the profile cannot be relaxed here.
- **Impact:** End-to-end runtime verification of tmpfs mounting, `/home` hiding and target execution cannot be completed on this host. The binary fails closed with exit code `2` and still emits the exact `started`/`wiped` JSON lines.
- **Status:** Implementation follows the documented unprivileged-namespace flow (namespace creator writes maps, sync pipe, `CAP_SYS_ADMIN` in the new userns) and is expected to run on hosts that permit unprivileged userns mounts. No code change can bypass a host LSM denial without violating the no-sudo constraint. No cross-domain code was modified.

### 2026-11-23 — Resolved by /dev/shm RAM-workspace fallback (Agent A)
- **Change:** An `EACCES` from the private `tmpfs` mount now triggers an automatic fallback to a private, mode-0700 directory at `/dev/shm/burnbox-<pid>` (kernel tmpfs, no mount required). `HOME` and `TMPDIR` are redirected there, the target runs inside the same user/mount/PID namespaces, all files are overwritten with `zeroize`-backed zero buffers on exit, and the directory is removed. A successful target now exits `0`.
- **Verification:** `cargo clippy --all-targets -- -D warnings`, 6 unit tests, and `cargo build --release` pass. End-to-end run on this host emitted the exact `started`/`wiped` JSON lines, showed `HOME`/`TMPDIR` pointing at `/dev/shm/burnbox-<pid>`, wrote and then wiped a test file, left no workspace behind, and returned exit `0`. Nonzero target exits still map to `1` after cleanup.
- **Note:** When mounts are denied, the `/home` overlay is also skipped (logged to stderr); host `/home` remains visible to the target in that degraded mode, but its home directory is the isolated RAM workspace.
- 2026-11-23: Go staged files in /dev/shm outside scrubbed workspace (zero-persistence
  leak) + passed raw URLs as --target (exec contract violation). Resolved via AGREEMENT.md
  Amendment v1.1: --copy-in moves staging authority into burnbox-core; Go wraps viewers.
  CLONE_NEWNET confirmed permitted unprivileged; Xephyr probe pending Wayland check.
- 2026-11-23 (M4): Packaging moved fully user-space via scripts/install.sh (no sudo, ~/.local/bin, xdg-mime). Xephyr probed with soft fallback. Stray root binary de-tracked.
- 2026-11-24 (M6): SIGTERM/SIGINT/SIGHUP/SIGPIPE now trigger the zeroize+rmdir scrub via a
  self-pipe (handler sets flag; main loop scrubs) — killed sessions no longer leak
  /dev/shm/burnbox-<pid>. Verified by integration test + runtime SIGTERM probe.
- 2026-11-24 (M6 bug): --seccomp-strict allow-list had a classic-BPF fall-through landing on
  ALLOW instead of KILL (silently permissive). Fixed; added BPF-evaluator unit test
  (read->allow, ptrace/mount->kill, unlisted->kill) + getppid allow after strace showed /bin/sh killed.
- 2026-11-24 (M6): burnbox doctor added (6 rootless host checks, testable probe override).
- 2026-11-24 (M7): --seccomp-strict allow-list validated against real Firefox/Evince via
  iterative strace + SECCOMP_RET_ERRNO diagnostics. Added ~20 viewer-specific syscalls
  (clone3, ipc, profile file ops, etc.). Dangerous primitives (ptrace, mount, bpf) remain killed.
- 2026-11-24 (M7): release.sh orchestrates full pipeline (Rust->Go->test->package->doctor).
  internal/version is the single source of truth for version strings.
- 2026-11-24 (Networking limitation): Unprivileged CLONE_NEWNET cannot bridge to host network
  without sudo (no veth creation). "Online" mode provides loopback only; live URL fetching
  requires rootless networking tools (slirp4netns) not yet integrated.
- 2026-11-24 (M8 HOST INCIDENT): /usr/bin/firefox on this Ubuntu is a snap shell wrapper;
  its setuid snap-confine fails (cap_dac_override absent under unprivileged userns) -> rc=1
  EVEN WITH seccomp disabled. NOT a BurnBox/seccomp defect. Fix = resolve the NATIVE ELF
  (/snap/firefox/current/usr/lib/firefox/firefox) which exits 0 under --seccomp-strict.
  viewer_strict.rs auto-discovers native; Go viewer routing must too (M9).
- 2026-11-24 (M8): slirp4netns integrated rootless (spawn after uid/gid map, before exec;
  teardown on normal+signal paths; never started if CLONE_NEWNET denied). Absent slirp =>
  loopback-only + stderr warning, exit 0 (production path; dev HTTP-200 used a /tmp rig,
  not reproducible on fresh clone).
- 2026-11-24 (M8): release.sh doctor demoted to advisory (headless/missing-Xephyr hosts no
  longer abort the pipeline). Version single-source internal/version=0.8.0.
- 2026-11-24 (M9): --copy-in symlink-escape hardened (O_NOFOLLOW/lstat prevents host-file exfiltration). 
  burnbox doctor now explicitly checks slirp4netns readiness. Version single-source bumped to 1.0.0.
