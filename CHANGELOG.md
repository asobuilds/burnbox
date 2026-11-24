# Changelog

All notable changes to BurnBox will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0),
and this project adheres to [Semantic Versioning](https://semver.org/).

## Unreleased

### Added
- `burnbox doctor` subcommand with 6 host readiness checks (AppArmor, CLONE_NEWNET, Xephyr, Seccomp, Viewer, /dev/shm)
- `burnbox --version` flag printing version from ldflags
- Packaging script `scripts/package.sh` creating .tar.gz and .deb outputs
- XDG desktop entry `packaging/burnbox.desktop` for Open With integration
- Zero-sudo install script `scripts/install.sh` registering MIME types and desktop entries

### Changed
- Version now injected via ldflags: `-X burnbox/internal/version.Version`
- CLI banner updated with usage tips and doctor recommendation
- Packaging script recommends running `burnbox doctor` post-install

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