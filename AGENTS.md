# Agent Directives & Role Boundaries

## Mandatory Startup Protocol
Before reading code or executing commands, every agent MUST:
1. Read `AGREEMENT.md` to check active file locks and interface contracts.
2. Read `RESOLUTION.md` to check active errors, blockers, or rollback rules.
3. Run `git status` to verify the workspace state.

## Strict Domain Ownership (Zero-Overlap Rule)
- **Agent A — OpenCode (Systems & Security Engineer):**
  - **Allowed Write Paths:** `sandbox-core/**` (Rust crates, Cargo.toml, namespace/seccomp/tmpfs logic).
  - **Read-Only Paths:** `cmd/**`, `internal/**`, `go.mod`.
- **Agent B — Gemini CLI (Orchestrator & UX Engineer):**
  - **Allowed Write Paths:** `cmd/**`, `internal/**`, `go.mod`, `scripts/**`, `packaging/**`.
  - **Read-Only Paths:** `sandbox-core/**`.
- **Shared Governance Files:** `AGREEMENT.md` and `RESOLUTION.md` may be appended to by either agent using atomic section updates, never overwriting another agent's active entry.

## Environment Constraint
- The host user (`student`) does not have `sudo` privileges. All sandboxing in `sandbox-core` MUST use **Linux Unprivileged User Namespaces** (`CLONE_NEWUSER`) combined with `CLONE_NEWNS` and `CLONE_NEWPID` so no root privileges are required at runtime.

## Build & Verification Commands
- **Go Verification:** `go test ./... && go build -o bin/burnbox ./cmd/burnbox`
- **Rust Verification:** `cd sandbox-core && cargo clippy && cargo test && cargo build --release && cp target/release/burnbox-core ../bin/`
