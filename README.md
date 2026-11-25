# BurnBox: An Ephemeral, Rootless Sandbox for Mitigating Phishing and Local Data Exfiltration on Linux

**Version:** v1.0.2 · **Status:** Production-ready · **License:** MIT · **Platform:** Linux

## Title & Abstract

**Title:** BurnBox — an ephemeral, rootless sandbox for mitigating phishing and local data exfiltration on Linux.

This study presents the design and implementation of BurnBox, a two-engine security tool that opens untrusted files and URLs inside a disposable, RAM-only Linux environment without `sudo`. The Go orchestrator resolves viewers, loads user defaults, manages display isolation and sessions, and delegates execution to the Rust `burnbox-core`. The core creates unprivileged user, mount, PID and network namespaces, applies a seccomp-BPF syscall policy, stages `--copy-in` files inside the workspace, and overwrites (`zeroize`s) and removes that workspace on **every** exit path — normal completion, signal, wall-clock timeout or error. On hosts where AppArmor denies private `tmpfs` mounts, BurnBox falls back to a private `/dev/shm` workspace (kernel `tmpfs`) instead of asking for elevated privileges. The result is a lightweight, verifiable and zero-persistence workflow for journalists, system administrators and everyday desktop users who need to inspect hostile artifacts on a Linux workstation. The documented CLI/JSON contract, reproducible release pipeline and honest acceptance tests make the security claims auditable rather than nominal.

## 1. Background of the Study

Modern Linux desktop environments prioritise usability over strict isolation. Opening an email attachment, a downloaded PDF or a suspicious hyperlink normally executes the viewer with the user's full privileges: access to `$HOME`, SSH keys, browser profiles, the whole filesystem and the host network. A malicious document can therefore read local secrets or establish persistence before the user notices anything.

Traditional defences are poorly matched to this threat model. Virtual machines and container runtimes provide strong boundaries, but they require privileged daemons or `sudo`, consume substantial memory and disk, and leave persistent images, snapshots and caches behind. Mandatory-access-control profiles (for example AppArmor or SELinux) are effective but require policy authoring and system administration that ordinary users cannot reasonably perform per attachment.

Linux unprivileged namespaces change this picture: a regular user can create a user namespace, a mount namespace, a PID namespace and a network namespace, then install a seccomp-BPF filter before executing a target. BurnBox builds on those primitives and addresses the practical obstacles that prevent them from being a turnkey desktop sandbox — notably host LSM denials, snap wrapper incompatibilities and teardown races.

## 2. Statement of the Problem

The central problem is the absence of a **lightweight, unprivileged and zero-persistence** sandbox for routine desktop artifact handling.

- **Phishing and malware:** a crafted link or document can exfiltrate credentials, encrypt local files or install a foothold when opened directly by the host viewer.
- **Privilege gap:** existing isolation tools typically need root, a daemon or complex per-application profiles, violating the principle of least privilege and excluding non-administrative users.
- **Persistence gap:** even "disposable" sessions often write to disk, leaving caches, temporary files or forensic traces after the viewer exits.
- **Host-policy friction:** on Ubuntu-like hosts, AppArmor's `unprivileged_userns` profile denies `mount(2)` inside new namespaces; a naive implementation simply fails instead of degrading safely.
- **Viewer and network caveats:** `/usr/bin/firefox` is often a `snap-confine` shell wrapper that cannot work inside an unprivileged namespace, and an unprivileged network namespace cannot bridge to the host network without a rootless userspace stack.
- **Teardown correctness:** killing a session must not bypass workspace scrubbing. Earlier timeout and signal paths leaked `/dev/shm/burnbox-*` directories, breaking the zero-persistence claim.

BurnBox evaluates whether these constraints can be reconciled in a single user-space tool while preserving a strict machine-readable interface between its components.

## 3. Objectives of the Project

1. **Strict isolation.** Combine `CLONE_NEWUSER`, `CLONE_NEWNS`, `CLONE_NEWPID` and `CLONE_NEWNET`, a private RAM workspace, hiding of the host `/home`, and seccomp-BPF (a default deny-list plus an opt-in `--seccomp-strict` allow-list). An optional `--fs-jail` uses `pivot_root` with a minimal read-only rootfs.
2. **Zero persistence.** Keep all session files in `tmpfs` or `/dev/shm`, stage `--copy-in` inputs mode `0600` inside that boundary, and call the same `zeroize_and_remove_workspace()` scrub with a `wiped` status line on normal, signal, timeout and error exits.
3. **Unprivileged execution.** Run entirely as the invoking user: no `sudo`, no setuid helpers, no privileged mounts. Host denials produce explicit warnings and safe fallbacks, never an escalation request.
4. **Verifiable security.** Maintain an authoritative CLI/JSON contract in `AGREEMENT.md`, a `burnbox doctor` host-readiness report, integration tests that assert real artifacts (or honest SKIPs), and a release pipeline that fails on stale engines or version drift.
5. **Practical desktop adoption.** Provide native `.deb` and portable `.tar.gz` artifacts, an Open-With handler for common document and link MIME types, and defaults that can be tuned without editing the code.

## 4. Methodology and Architecture

### Two-engine design

- **Orchestrator (Go, `cmd/` and `internal/`):** parses `burnbox open|doctor|list|kill`; loads defaults from `$BURNBOX_CONFIG`, `~/.config/burnbox/config.toml` or `config.json`; resolves the native viewer (`BURNBOX_VIEWER` → native Firefox ELF → native Chromium/Chrome → `xdg-open`, skipping `#!`/snap shims via `isNativeELF()`); probes Xephyr for an isolated `:99` display with a host-`DISPLAY` fallback; locates `burnbox-core` via `PATH` → `~/.local/bin` → `./bin`; streams the core's stdout to the terminal and parses its two JSON lines into user-facing messages.
- **Sandbox core (Rust, `sandbox-core/`):** creates the namespaces and a sync pipe, writes uid/gid maps, mounts the RAM workspace (or falls back to `/dev/shm/burnbox-*`) and a private `/home`, starts `slirp4netns` when available, installs seccomp-BPF, forks the target, and owns all cleanup paths.

### Execution contract

Per `AGREEMENT.md` (Amendment v1.1), `--target` is **always an executable path**; viewer arguments follow `--`. Files and URLs are never passed raw as `--target`:

```
burnbox-core --mode <offline|online> --ram-mb <size> --target <executable> [--] [args...]
```

Stdout is exactly two JSON lines, in order:

```
{"status": "started", "pid": <int>, "mount_point": "<path>"}
{"status": "wiped",   "pid": <int>, "mount_point": "<path>"}
```

All diagnostics go to stderr. Exit codes are `0` clean, `1` runtime error, `2` namespace/setup failure and `3` seccomp/security violation. `burnbox open` passes the core's stdout through unchanged so both lines stay visible.

### Session lifecycle

1. Go resolves the viewer/config/display and invokes the core with `--mode`, `--ram-mb`, `--target` and `--` arguments.
2. The core creates a private RAM workspace, copies any `--copy-in` files into it, and redirects `HOME`/`TMPDIR` there.
3. The namespace supervisor applies rlimits (`RLIMIT_NPROC`, `RLIMIT_AS`, and `RLIMIT_CPU` with `--timeout`), the seccomp filter and, if requested, the `pivot_root` jail; then it execs the target.
4. Online sessions attach `slirp4netns` for user-mode TCP/IP when available; offline sessions assert that only loopback is visible.
5. Teardown — whether normal, signal-driven (`SIGTERM`/`SIGINT`/`SIGHUP`/`SIGPIPE` via a self-pipe) or a wall-clock timeout owned by the parent — stops the network, zeroizes every file, removes the workspace, emits `wiped` and exits with the mapped code.

### Release and verification method

`scripts/release.sh` is the single no-sudo pipeline: derive the version from `internal/version/version.go`; build and publish `burnbox-core`; assert fresh core flags (`--fs-jail`, `--timeout`); build the Go binary and enforce a binary-versus-source version-drift kill-switch; run `go test ./...`; package `.deb` and `.tar.gz` with dynamic `dpkg --print-architecture`; generate `release/SHA256SUMS`; and run `burnbox doctor` as an advisory check. GitHub Actions validates the matrix `ubuntu-22.04`/`ubuntu-24.04` × `amd64`/`arm64` on native arm64 runners.

## 5. Complexities Involved

- **AppArmor and tmpfs fallback (M2/M11):** `mount("tmpfs", ...)` returns `EACCES` under Ubuntu's `unprivileged_userns` profile. The core falls back to a mode-0700 `/dev/shm/burnbox-<pid>` directory — already kernel tmpfs — and logs the degradation. The `/home` overlay is skipped in that mode, but `HOME` still points into the isolated workspace.
- **Seccomp fall-through and viewer compatibility (M6/M7):** a classic-BPF allow-list initially defaulted to ALLOW on unlisted syscalls; it was corrected to default KILL and covered with a BPF evaluator test. Iterative `strace` discovery added viewer-safe syscalls while keeping `ptrace`, `mount` and `bpf` killed.
- **Snap wrapper incompatibility (M8/M9):** `/usr/bin/firefox` is frequently a shell shim that starts `snap-confine` and fails with `cap_dac_override not found`. Viewer discovery checks the ELF magic and rejects `#!` wrappers, preferring the native Firefox ELF under `/snap/firefox/current/usr/lib/firefox/firefox`.
- **Symlink escape prevention (M9):** staged `--copy-in` inputs are opened with `O_NOFOLLOW`/`lstat` checks; direct and intermediate symlinks are rejected so host files cannot be read through the sandbox boundary.
- **Signal-safe scrubbing (M6):** an async handler must not allocate or call complex cleanup routines; it only sets an atomic flag and writes one byte to a self-pipe, waking `poll()` so the parent can scrub safely.
- **Timeout scrubbing (M11.1):** the wall-clock deadline moved from the supervisor to the parent. On expiry the parent `SIGKILL`s the PID-namespace supervisor, reaps it with a bounded `WNOHANG` loop, stops `slirp4netns`, scrubs the workspace, emits `wiped` and exits `1` — closing the `/dev/shm` leak.
- **AppImage pivot_root guard (M12):** AppImage payloads under `/tmp/.mount_*` are outside the static `/usr`/`/lib`/`/lib64`/`/bin` bind list. `--fs-jail` detects that layout from the target, `APPDIR` or the current executable and falls back to the overlay with a warning rather than hiding the viewer's own libraries.
- **Architecture and test hygiene (M12):** libc maps `io_uring_setup/enter/register` to `425/426/427` on both x86_64 and aarch64; `test_io_uring_denied_on_all_arches` asserts deny-list membership on every architecture and locks the ABI numbers on both. Shared-`/dev/shm` leak assertions now compare against a baseline so concurrent agents' live sessions do not create false failures.
- **Release integrity (M11.1/M12):** the version is read from source, never injected with `-ldflags`; the pipeline refuses to package a stale core or a mis-versioned binary, and CI covers both Debian-family architectures.

## 6. Significance of the Study

- **Journalists and investigators** can open hostile attachments or leaked documents without exposing their source's files, browser cookies or SSH keys to the artifact.
- **System administrators** get a rootless, scriptable triage tool that does not alter host policy, needs no daemon and fits into existing `.desktop`/MIME workflows.
- **Everyday users** receive a simple "Open With BurnBox" action plus `burnbox doctor`, so safety does not depend on understanding namespace internals.
- **Privacy-conscious users** benefit from zero on-disk residue: every session file is overwritten and removed, including when the session is interrupted or timed out.
- **Developers and researchers** get a documented inter-process contract, adversarial integration tests, a reproducible release pipeline and a concrete base for cross-platform backends. The study demonstrates that useful isolation can be built from unprivileged kernel primitives alone, with honest fallbacks where host policy refuses them.

## 7. Implementation

### Installation

**Debian/Ubuntu package** (system-wide integration; requires administrator rights at install time only):

```bash
sudo apt install ./burnbox_1.0.2_amd64.deb   # use _arm64.deb on ARM hosts
```

**Portable tarball** (zero-sudo, installs to `~/.local/bin`):

```bash
mkdir burnbox && tar -xzf burnbox_1.0.2_amd64.tar.gz -C burnbox
cd burnbox && ./install.sh
```

The tarball and `.deb` both contain `bin/burnbox` (Go) and `bin/burnbox-core` (Rust). `install.sh` copies them into `~/.local/bin`, registers the desktop entry and sets BurnBox as the default handler for PDF, ZIP, HTML, DOCX, `x-scheme-handler/http` and `x-scheme-handler/https`; no `sudo` is required. `release/SHA256SUMS` provides hashes for verifying downloaded artifacts.

### Command-line usage

```bash
burnbox --version
burnbox doctor                         # 7 host readiness checks

# Local file: staged inside the RAM workspace via --copy-in
burnbox open report.pdf --offline --copy-in report.pdf --ram 256

# URL: wrapped in a native viewer executable, no raw URL as --target
burnbox open https://example.com --online
burnbox open https://example.com --screenshot /tmp/proof.png

# Force mode regardless of config defaults
burnbox open report.pdf --offline
burnbox open https://example.com --online
```

`--offline`/`-o` and `--online` select the mode; `--ram`/`-r` sets the workspace size in MiB; `--copy-in` is repeatable; `--screenshot` requests a viewer screenshot artifact. Advanced core flags are available on `burnbox-core` directly: `--fs-jail`, `--seccomp-strict` and `--timeout <SECS>`.

### Session management

```bash
burnbox list                 # active/stale RAM workspaces with their PIDs
burnbox kill <pid|burnbox-pid|path>   # SIGTERM to the parent → safe scrub
```

`kill` only targets a PID that owns a live `/dev/shm/burnbox-<pid>` workspace; the signal triggers the same zeroize-and-remove path as a normal exit, and a stale session is reported for manual cleanup rather than killed blindly.

### Right-click integration

`packaging/burnbox.desktop` declares `Exec=burnbox open %F` and the supported MIME types. `install.sh` registers it per user with `xdg-mime`/`update-desktop-database`; the `.deb` post-install registers it system-wide. Right-click a document or link → **Open With → BurnBox Sandbox**.

### Configuration file

Defaults are loaded from the first available source: `$BURNBOX_CONFIG`, `~/.config/burnbox/config.toml`, then `~/.config/burnbox/config.json`. Missing or malformed files are ignored safely; CLI flags always win.

```toml
# ~/.config/burnbox/config.toml
mode = "offline"   # offline | online (built-in default: online)
ram_mb = 256       # positive integer (built-in default: 512)
```

### Building and verifying from source

```bash
go test ./... && go build -o bin/burnbox ./cmd/burnbox
./scripts/release.sh    # Rust → Go → tests → package → SHA256SUMS → advisory doctor
./scripts/acceptance.sh # real artifact / non-leak assertions with honest SKIPs
```

## 8. Scope and Limitations

- **Linux only.** The implementation depends on Linux namespaces and seccomp-BPF. Hosts must permit unprivileged user namespaces; no Windows, macOS or Android backend exists yet.
- **Host policy degrades isolation.** Where AppArmor denies private mounts, the `/dev/shm` fallback keeps the workspace RAM-only but the host `/home` overlay is skipped, so host files remain visible unless an `--fs-jail` succeeds. This is logged to stderr and never hidden.
- **Live URLs need `slirp4netns`.** Without it, an unprivileged network namespace is loopback-only and the core warns: `Warning: slirp4netns not found; network isolation is strict loopback only. Install slirp4netns for live URL fetching.` `burnbox doctor` reports the same capability informationally.
- **Snap wrapper caveat.** `snap-confine` requires capabilities unavailable inside the sandbox; BurnBox skips `#!` snap shims and prefers native ELF viewers. A host with only a snap browser may need a native Firefox/Chromium or `BURNBOX_VIEWER` override for GUI URLs.
- **Xephyr is optional.** If `/usr/bin/Xephyr` is absent or fails to start, the session uses the host `DISPLAY`, reducing display isolation (stderr warns).
- **`--fs-jail` is opt-in and best-effort.** `pivot_root`, fresh `hidepid=2` procfs and the static bind list may be denied by the host LSM; AppImage `/tmp/.mount_*` payloads deliberately trigger the overlay fallback. In these cases the tool warns and continues rather than failing closed on a host-policy denial.
- **Store-and-forward only for trusted input.** `--copy-in` deliberately rejects symlinks; it does not follow host links into the workspace.

## 9. Future Work

The cross-platform blueprint expands the same research problem to other isolation substrates:

- **M13 — Backend Abstraction:** extract an `IsolationBackend` interface from the Rust core so namespace, container and VM strategies can be selected and tested independently while preserving the two-line JSON contract.
- **M14 — WSL2:** bring the rootless Linux workflow to Windows users through Windows Subsystem for Linux, adapting display hand-off and host-file staging.
- **M15 — macOS Seatbelt:** implement a `sandbox-exec`/Seatbelt backend with an equivalent RAM-scrub lifecycle for macOS attachments and links.
- **M16 — Android pKVM:** explore per-app UID isolation plus protected KVM for a mobile implementation of the zero-persistence model.
- **M17 — Firecracker/gVisor:** add microVM and user-space-kernel backends for serverless and CI threat triage at higher isolation strength than namespaces alone.
