# BurnBox — Ephemeral Anti-Phishing & Privacy Sandbox for Linux

BurnBox is a lightweight, zero-persistence Linux sandbox designed to protect everyday users from phishing links, malicious attachments, and local data leaks.

## Architecture
- **Orchestrator & UI (`cmd/`, `internal/`):** Written in **Go**. Handles CLI commands, input validation, desktop integration, and process lifecycle.
- **Sandbox & Wipe Engine (`sandbox-core/`):** Written in **Rust**. Uses Linux Unprivileged User/Mount/PID/Network Namespaces (`CLONE_NEWUSER`, `CLONE_NEWNS`, `CLONE_NEWPID`, `CLONE_NEWNET`), `tmpfs` RAM mounts, Seccomp filters, and `zeroize` memory scrubbing.

## Multi-Agent Workflow
This repository is built concurrently by **OpenCode** (Rust Core & Security Agent) and **Gemini CLI** (Go Orchestrator & Integration Agent). All agents must read `AGENTS.md`, `AGREEMENT.md`, and `RESOLUTION.md` before modifying any file.

## Quick Start
```bash
# Build
go build -ldflags "-X burnbox/internal/version.Version=1.0.0" -o bin/burnbox ./cmd/burnbox

# Run host readiness check
./bin/burnbox doctor

# Open a file offline
./bin/burnbox open document.pdf --offline --copy-in document.pdf

# Open a URL (requires slirp4netns for live fetching)
./bin/burnbox open https://example.com

# Install (zero-sudo, to ~/.local/bin/)
./scripts/install.sh

# One-command release
./scripts/release.sh

# End-to-end acceptance test
./scripts/acceptance.sh
```

## Milestone Status (M1–M10 shipped, M11 in progress)
- **M1** Go orchestrator MVP — shipped
- **M2** Rust namespace + RAM zero-wipe core — shipped
- **M3** Offline `CLONE_NEWNET` · Seccomp-BPF · `--copy-in` · smart routing · Xephyr probe — shipped
- **M4** Desktop integration (`.desktop` + `xdg-mime`) + user-space `install.sh` — shipped
- **M5** Packaging artifacts (.tar.gz + .deb) + `--version` flag — shipped
- **M6** Signal-safe scrub · `--seccomp-strict` allow-list · `burnbox doctor` — shipped
- **M7** Release pipeline (`release.sh`) + CHANGELOG — shipped
- **M8** Rootless networking (slirp4netns) + acceptance test (`acceptance.sh`) — shipped
- **M9** Viewer auto-discovery (snap/native/alt-browser) + v1.0.0 — shipped
- **M10** Tarball `bin/` layout + robust core path + desktop MIME fix + postinst root fix — shipped
- **M11** Viewer auto-discovery hardening + acceptance.sh artifact verification — in progress

## Known Limitations
- **Networking in sandbox:** Unprivileged network namespaces (`CLONE_NEWNET`) cannot reach the internet without `sudo` or `slirp4netns`. Without slirp4netns, "online" mode provides loopback only. Live URL fetching requires `slirp4netns` (or `BURNBOX_SLIRP4NETNS` env var).
- **Snap Firefox shim:** On Ubuntu hosts where `/usr/bin/firefox` is a snap wrapper, sandboxed launch fails due to snap-confine capability requirements. BurnBox auto-discovers native Firefox/Chromium ELF binaries and falls back to `xdg-open` only as last resort. Use a native Firefox/Chromium build for full GUI support.
- **Viewer auto-discovery:** `findViewer()` probes in order: `BURNBOX_VIEWER` env override → native Firefox ELF paths → `/usr/bin/firefox` (skipped if snap shim) → native Chromium/Chrome → `xdg-open` (last resort). Snap shims are detected by reading the file header (rejects `#!` scripts and `snap-confine` wrappers).

## CLI Reference
```
burnbox open <file-or-url> [--offline] [--ram <mb>] [--copy-in <host_path>...] [--screenshot <path>]
burnbox open <url> [--offline] [--ram <mb>]
burnbox doctor
burnbox --version
```

## v1.0.1 — hotfix release (2026-11-25)
Fixes to v1.0.0 found by a clean-room audit (build-from-source gate, not dev-machine assumptions):
- **Packaging:** the portable `.tar.gz` now ships `bin/burnbox-core` (it was missing → tarballs were non-functional); artifacts are version-named (`burnbox_1.0.1_amd64.{deb,tar.gz}`) and `release/` is wiped before each build (no stale versions).
- **Desktop:** `x-scheme-handler/http` + `https` added (right-click a *link* → Open With → BurnBox now actually works); invalid `application/x-html` removed.
- **Core path:** `burnbox` locates `burnbox-core` via PATH → `~/.local/bin` → `./bin` (3-tier), so it works when launched from the desktop/`.desktop`, not only from the repo dir.
- **`.deb`:** `postinst` no longer writes `$HOME` (it runs as root) — desktop/MIME now register system-wide correctly.
- **Security:** `io_uring_*` denied in the default seccomp policy (async-I/O syscall-visibility bypass); opt-in `--fs-jail` (pivot_root minimal rootfs), fresh `hidepid=2` `/proc`, `RLIMIT_NPROC/AS/CPU`, and `--timeout` (now scrubs the workspace on kill).
- **Viewer auto-discovery:** `findViewer()` skips the broken snap shim (`/usr/bin/firefox`) and prefers a native Firefox/Chromium ELF.
- **Honest tests:** `acceptance.sh` requires a real screenshot artifact and a host-file non-leak assertion; it **SKIPs** (never fakes PASS) when slirp4netns/native viewer are absent.
- **Version single-source:** `release.sh` reads the version from `internal/version/version.go` (no `-ldflags` clobber) and **fails the pipeline** if binary ≠ source (drift kill-switch).

### Known limitations (honest)
- **Live URL fetching** still requires `slirp4netns` on PATH (or `BURNBOX_SLIRP4NETNS`); otherwise online mode is loopback-only with a warning. One-time host setup: `sudo apt-get install -y slirp4netns`.
- **`--fs-jail`** is opt-in and, on AppArmor-confined hosts (Ubuntu), `pivot_root` is denied → graceful overlay fallback. The jail is proven by tests conditional on mount support; verify on a host that allows unprivileged `pivot_root` (e.g. Fedora/Arch, or Ubuntu with `unprivileged_userns_clone` + permissive profile).

### Milestone status (single source of truth)
- **M1–M10** shipped · **M11 / M11.1** shipped (auto-discovery, honest acceptance, fs-jail/proc/rlimits/timeout+scrub, version single-source)
- **v1.0.1** tagged & released
- **M12** next — CI matrix, AppImage, GPG signing + SHA256SUMS, config file, Wayland/Xephyr live GUI smoke test, `burnbox list/kill`
