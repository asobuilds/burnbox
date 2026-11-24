# BurnBox — Ephemeral Anti-Phishing & Privacy Sandbox for Linux

BurnBox is a lightweight, zero-persistence Linux sandbox designed to protect everyday users from phishing links, malicious attachments, and local data leaks.

## Architecture
- **Orchestrator & UI (`cmd/`, `internal/`):** Written in **Go**. Handles CLI commands, input validation, desktop integration, and process lifecycle.
- **Sandbox & Wipe Engine (`sandbox-core/`):** Written in **Rust**. Uses Linux Unprivileged User/Mount/PID/Network Namespaces (`CLONE_NEWUSER`, `CLONE_NEWNS`, `CLONE_NEWPID`, `CLONE_NEWNET`), `tmpfs` RAM mounts, Seccomp filters, and `zeroize` memory scrubbing.

## Multi-Agent Workflow
This repository is built concurrently by **OpenCode** (Rust Core & Security Agent) and **Gemini CLI** (Go Orchestrator & Integration Agent). All agents must read `AGENTS.md`, `AGREEMENT.md`, and `RESOLUTION.md` before modifying any file.

## Milestone 4 — Desktop integration & packaging (shipped 2026-11-23)
Run `./scripts/install.sh` (no sudo) to copy binaries to `~/.local/bin/` and register BurnBox as the default handler for PDFs and HTTP/HTTPS links via `xdg-mime`. Right-click any suspicious file/link in your file manager → **Open With → BurnBox**.

### Milestone status
- **M1** Go orchestrator MVP — shipped
- **M2** Rust namespace + RAM zero-wipe core — shipped
- **M3** Offline `CLONE_NEWNET` · Seccomp-BPF · `--copy-in` · smart routing · Xephyr probe — shipped
- **M4** Desktop integration (`.desktop` + `xdg-mime`) + user-space `install.sh` — **shipped**
- **M5** Seccomp allow-list hardening + packaging artifacts (.deb/AppImage) — next

## Milestone 6 — Signal-safe scrub + strict seccomp + doctor (shipped 2026-11-24)
- **No leak on kill:** `SIGTERM`/`SIGINT`/`SIGHUP`/`SIGPIPE` now run the zeroize+rmdir scrub (self-pipe pattern), so an interrupted session never leaves `/dev/shm/burnbox-*` behind.
- **`--seccomp-strict`:** switches the filter from the deny-list to a ~120-syscall viewer allow-list (kill-by-default); logs a GUI-compat warning to stderr.
- **`burnbox doctor`:** rootless host self-check (AppArmor fallback, `CLONE_NEWNET`, Xephyr, seccomp, viewer-on-PATH, `/dev/shm` writable) — run it after install.

### Milestone status
- **M1–M5** shipped
- **M6** Signal-safe scrub · `--seccomp-strict` allow-list · `burnbox doctor` — **shipped**
- **M7** End-to-end GUI validation + reproducible release pipeline — next

## Milestone 7 — Strict seccomp validated + Release pipeline (shipped 2026-11-24)
- **Real GUI validation:** `--seccomp-strict` allow-list proven against native Firefox/Evince (exit 0, real screenshot). Dangerous syscalls still killed.
- **One-command release:** `./scripts/release.sh` builds, tests, packages, and runs doctor.
- **Version consistency:** `internal/version` is the single source of truth (overridden via ldflags).

### Known Limitations
- **Networking in sandbox:** Unprivileged network namespaces (`CLONE_NEWNET`) cannot reach the internet without `sudo` or `slirp4netns`. "Online" mode currently provides loopback only. Live URL fetching is limited until rootless networking is added.

### Milestone status
- **M1–M6** shipped
- **M7** Strict seccomp (Firefox-validated) · `release.sh` · CHANGELOG — **shipped**
- **M8** Rootless networking (slirp4netns) + final GUI acceptance test — next

## Milestone 8 — Rootless networking + acceptance (shipped 2026-11-24)
- **Live URLs (optional):** if `slirp4netns` is on PATH (or `BURNBOX_SLIRP4NETNS` set), `--mode online` gets user-space TCP/IP inside the netns — no sudo. Without it, graceful loopback-only + warning.
- **Strict mode proven:** `--seccomp-strict` allow-list validated against the **native** Firefox ELF (exit 0, real screenshot). `/usr/bin/firefox` is a broken snap shim on Ubuntu — see Known Limits.
- **One-command release:** `./scripts/release.sh` (doctor advisory) + `./scripts/acceptance.sh` (3/3).

### Known Limitations
- **Snap Firefox shim:** on Ubuntu hosts where `/usr/bin/firefox` is a snap wrapper, sandboxed launch fails with rc=1 (snap-confine capability issue, not seccomp). Use a native Firefox/Chromium build, or wait for M9 viewer auto-discovery.

### Milestone status
- **M1–M7** shipped
- **M8** slirp4netns rootless networking · strict-vs-native-Firefox proof · acceptance.sh · doctor-advisory release — **shipped**
- **M9** Viewer auto-discovery (snap/native/alt-browser) + live Wayland/Xephyr GUI smoke test — next
