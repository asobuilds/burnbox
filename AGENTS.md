# AGENTS.md — BurnBox Multi-Agent Governance

Two autonomous agents build this repo **concurrently** in split terminals inside
`~/projects/burnbox`. Strict domain ownership prevents write clashes.

## Roles & write scopes
| Agent | Role | Exclusive write scope |
|-------|------|-----------------------|
| **A — OpenCode** | Systems & Security Engineer | `sandbox-core/**` |
| **B — Gemini CLI** | Orchestrator & UX Engineer | `cmd/**`, `internal/**`, `go.mod`, `scripts/**`, `packaging/**` |
| **Director / Architect** | Product + coordination | root governance (`AGENTS.md`, `AGREEMENT.md`, `RESOLUTION.md`), `README.md`, `.gitignore` |

`bin/burnbox-core` is produced by Agent A (`cp target/release/...`); `bin/burnbox` by
Agent B (`go build -o`). `bin/` and `target/` are gitignored — never commit binaries.

## Startup protocol (every session)
1. Read `AGREEMENT.md` (CLI/JSON contract, **Amendment v1.1**), `RESOLUTION.md` (concurrency + host incidents), and this file.
2. Touch **only** your scope. Never edit the other agent's files or governance/README.
3. Respect host constraints: **no sudo**, AppArmor `unprivileged_userns` (tmpfs `mount` denied ⇒ `/dev/shm` fallback), `CLONE_NEWNET` allowed, `Xephyr` present.
4. Run your verification block; commit with your prefix (`opencode:` / `go:`) only when green.

## Verification commands
**Agent A (Rust):**
cd sandbox-core && cargo clippy --all-targets -- -D warnings && cargo test && cargo build --release && cp target/release/burnbox-core ../bin/

**Agent B (Go):**
go test ./... && go build -o bin/burnbox ./cmd/burnbox

## Shared contract (authoritative copy in AGREEMENT.md)
- `--target` MUST be an executable path; viewer args go after `--`. Never pass a raw URL/file as `--target`.
- `--copy-in <host_path>` (repeatable) stages files **inside** the RAM workspace (mode `0600`) before exec; they fall within the `zeroize` scrub.
- Stdout = exactly two JSON lines (`started`, `wiped`); all diagnostics on stderr.
- Exit codes: `0` clean · `1` runtime · `2` namespace/setup · `3` seccomp.

## Concurrency rules (see RESOLUTION.md)
- File-lock table governs shared paths; `bin/` writes are serialized by build order (Rust first publishes `burnbox-core`, then Go runs E2E against it).
- Host incidents (AppArmor `EACCES` fallback; stale-`/sys` netns caveat ⇒ `if_nameindex()`/`/proc/net/dev` authoritative) are logged in `RESOLUTION.md` and must not be "fixed" by re-introducing privileged mounts.

## Milestone status
- [x] M1 Go orchestrator MVP
- [x] M2 Rust namespace + RAM zero-wipe core
- [x] M3 Offline `CLONE_NEWNET` assertion · Seccomp-BPF · `--copy-in` staging · smart URL/file routing · Xephyr display isolation
- [ ] M4 Desktop integration + packaging

## Milestone 4 status (2026-11-23) — shipped
- Agent B delivered `scripts/install.sh` (user-space `~/.local/bin` + `xdg-mime`, no sudo) and the `cmd/burnbox/main.go` banner.
- Agent A re-verified `sandbox-core` and republished `../bin/burnbox-core`.
- Stray root-level `burnbox` binary de-tracked; `/burnbox` added to `.gitignore`.

## Milestone 6 status (2026-11-24) — shipped
- Agent A: signal-safe scrub (self-pipe) + real --seccomp-strict allow-list + BPF fall-through KILL fix + BPF-evaluator test. Only sandbox-core/** touched.
- Agent B: burnbox doctor (6 checks, testable probe override) + banner wiring + script recommendations.

## Milestone 7 status (2026-11-24) — shipped
- Agent A: Strict allow-list validated against real Firefox (strace-based discovery). viewer_strict.rs integration test added.
- Agent B: release.sh orchestrator, CHANGELOG.md, internal/version single source.

## Milestone 8 status (2026-11-24) — shipped
- Agent A: src/network.rs slirp4netns (rootless, guarded, dual teardown) + BURNBOX_SECCOMP_DIAG + native-ELF strict proof. Only sandbox-core/** touched.
- Agent B: acceptance.sh, release.sh doctor-advisory, version 0.8.0 single-source, open URL arg-parse fix.

## Milestone 9 status (2026-11-24) — shipped
- Agent A: sandbox-core symlink-escape hardening (O_NOFOLLOW), integration test added.
- Agent B: internal/version 1.0.0, doctor slirp check, CHANGELOG.md, release.sh manual tag output.
