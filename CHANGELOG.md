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
