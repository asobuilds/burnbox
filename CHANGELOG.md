# Changelog

All notable changes to BurnBox will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [1.0.0] - 2026-11-24

### Added
- `burnbox doctor` subcommand with 7 host readiness checks:
  - AppArmor unprivileged_userns (tmpfs mount denied => /dev/shm fallback is expected/OK)
  - CLONE_NEWNET permitted (via `unshare -U -n true`)
  - Xephyr display probe (binary present + basic probe)
  - Seccomp available (kernel seccomp-bpf support)
  - Viewer on PATH (firefox/chromium/xdg-open)
  - /dev/shm writable
  - slirp4netns for online URLs (PATH or BURNBOX_SLIRP4NETNS env var)
- `burnbox --version` flag printing version from single source of truth (`internal/version`)
- `scripts/package.sh` creating .tar.gz and .deb outputs with dynamic version
- `scripts/release.sh` single-command reproducible release orchestrator (doctor advisory)
- `scripts/acceptance.sh` end-to-end acceptance test (3 tests, final PASS/FAIL summary)
- XDG desktop entry `packaging/burnbox.desktop` for Open With integration
- Zero-sudo install script `scripts/install.sh` registering MIME types and desktop entries
- Single source of truth for version: `internal/version/version.go`

### Changed
- Version moved to single source of truth: `internal/version/version.go` (1.0.0)
- `scripts/package.sh` extracts version dynamically from `bin/burnbox --version`
- `scripts/release.sh` doctor check is advisory (does not fail the build)
- CLI banner updated with doctor subcommand and version tips
- `burnbox doctor` always PASSes slirp4netns check (informational; details indicate capability)

### Fixed
- URL routing bug: `burnbox open <url> --ram <mb>` now correctly passes the URL as the
  target argument (not a mangled flag value like "512") to xdg-open/firefox
- Argument parsing in `handleOpen()`: first non-flag positional argument is now
  correctly captured as the target, regardless of flag placement

## 0.8.0

### Added
- `scripts/release.sh` single-command reproducible release orchestrator
- `scripts/acceptance.sh` end-to-end acceptance test
- `burnbox doctor` subcommand (6 checks + version banner)
- `scripts/package.sh` dynamic version extraction from `bin/burnbox --version`

### Changed
- `burnbox doctor` made advisory in `scripts/release.sh` (does not fail build)
- Version bumped to 0.8.0 via `internal/version` single source

## 0.7.0

### Added
- `burnbox doctor` subcommand with 6 host readiness checks (AppArmor, CLONE_NEWNET, Xephyr, Seccomp, Viewer, /dev/shm)
- `burnbox --version` flag printing "BurnBox v0.7.0"
- `scripts/release.sh` single-command reproducible release orchestrator
- CHANGELOG.md for release tracking

### Changed
- Version moved to single source of truth: `internal/version/version.go`
- CLI banner updated with doctor subcommand and tips
- Packaging script recommends running `burnbox doctor` to verify host setup

## 0.6.0

### Added
- `burnbox doctor` subcommand with host readiness checks
- CLI banner with usage tips and doctor recommendation
- Packaging script creating .tar.gz and .deb outputs

### Changed
- Version injected via ldflags: `-X burnbox/internal/version.Version`

## 0.5.0

### Added
- `--version` flag printing "BurnBox v0.5.0"
- `scripts/package.sh` creating .tar.gz and .deb outputs
- Go test `TestDesktopFileValidation` validating desktop file content
- `cmd/burnbox/main.go` friendly usage banner with install tip

## [1.0.1] - 2026-11-25
### Fixed
- Portable `.tar.gz` now includes `bin/burnbox-core` (was omitted; tarballs were non-functional).
- `release/` wiped before each build; artifacts version-named (no stale 0.5.0–0.8.0).
- `.desktop` + `install.sh` register `x-scheme-handler/http|https`; removed invalid `application/x-html`.
- `burnbox` resolves `burnbox-core` via PATH → `~/.local/bin` → `./bin` (works from desktop launch).
- `.deb` postinst no longer references `$HOME` (runs as root) — system-wide desktop/MIME registration.
- `--timeout` now scrubs the RAM workspace and emits the `wiped` JSON line (was leaking /dev/shm).
- `release.sh` reads version from `internal/version/version.go`; no ldflags clobber; version-drift kill-switch.
### Added
- `io_uring_setup/enter/register` denied in default seccomp policy.
- Opt-in `--fs-jail` (pivot_root minimal rootfs), fresh `hidepid=2` /proc, `RLIMIT_NPROC/AS/CPU`.
- Viewer auto-discovery `isNativeELF()` — skips the snap shim, prefers native Firefox/Chromium ELF.
- Honest `acceptance.sh`: real screenshot artifact + host-file non-leak assertion; SKIP not fake PASS.

## [1.0.2] - 2026-11-25
### Added
- GitHub Actions CI matrix (ubuntu-22.04/24.04 × amd64/arm64) using native arm64 runners.
- Dynamic `dpkg --print-architecture` in packaging; `release/SHA256SUMS` for published `.deb`/`.tar.gz`.
- `burnbox list` / `burnbox ls` (active vs stale RAM workspaces) and `burnbox kill <pid|burnbox-pid|path>` (SIGTERM to parent → safe scrub → wiped).
- Config file defaults via `$BURNBOX_CONFIG`, `~/.config/burnbox/config.toml` or `config.json` (`mode`, `ram_mb`); CLI flags override them.
- AppImage-safe `--fs-jail` fallback for `/tmp/.mount_*` payloads; `test_io_uring_denied_on_all_arches` audit for x86_64/aarch64.

### Changed
- `burnbox open` now streams `burnbox-core` stdout through to the terminal, preserving the two contract JSON lines for the user; summaries are parsed from that same stream (no second core invocation).
- Release pipeline verifies freshly built core flags (`--fs-jail`, `--timeout`), checks binary-versus-source version drift, and keeps `burnbox doctor` advisory for headless CI.
- Shared-`/dev/shm` leak assertions in Rust integration tests compare against a baseline so concurrent agents' live sessions do not cause false failures, without weakening each session's own mount-point assertion.
- Documentation rewritten in the Nigerian Research Problem Approach: abstract, background, problem, objectives, methodology, complexities, significance, implementation, scope/limitations and M13–M17 roadmap.

### Fixed
- `burnbox kill` refuses PIDs without a live workspace and reports stale ones for manual cleanup instead of signalling an unrelated process; teardown remains the core's signal-safe zeroize+rmdir path.
- Timeout teardown remains parent-owned: SIGKILL supervisor, bounded reap, stop network, scrub, emit `wiped`, exit `1` (`sandbox-core/tests/timeout_scrub.rs`).
- Defaults from a missing/malformed config no longer break a run; invalid `mode`/`ram_mb` values are normalised.
