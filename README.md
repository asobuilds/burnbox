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
