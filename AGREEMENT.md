# Inter-Agent Agreement & Interface Contract

## 1. CLI Contract Between Go (Orchestrator) and Rust (`sandbox-core`)
Both agents agree to strictly adhere to this CLI interface so Go and Rust can be developed concurrently without breaking integration:

- **Binary Name:** `burnbox-core` (compiled to `./bin/burnbox-core`)
- **Invocation Syntax:**
  `burnbox-core --mode <offline|online> --ram-mb <size> --target <executable_path> [--] [target_args...]`
- **Exit Codes:**
  - `0`: Session ended cleanly and `tmpfs` memory was zeroed/unmounted.
  - `1`: General runtime error.
  - `2`: Namespace/sandbox setup failure (`CLONE_NEWUSER` / `tmpfs` error).
  - `3`: Security violation or Seccomp termination.
- **Standard Output (JSON status lines on startup and teardown):**
  `{"status": "started", "pid": <int>, "mount_point": "<path>"}`
  `{"status": "wiped", "pid": <int>, "mount_point": "<path>"}`

## 2. Active File Locks (Update Before Editing Shared Files)

| Agent | Locked Path | Task Description | Status |
| :--- | :--- | :--- | :--- |
| Agent A — OpenCode | `sandbox-core/**` | Rust sandbox core: userns/mount/pid namespaces, tmpfs RAM dir, /home hiding, JSON status lines, wipe on exit | LOCKED |
| Agent B — Gemini CLI | `cmd/**`, `internal/**`, `go.mod` | Orchestrator: CLI implementation, input validation, burnbox-core spawning, SIGINT handling | LOCKED |

## 3. Change Proposals
*(If an agent needs to alter the CLI contract above, log the proposal here and wait for confirmation before changing implementation.)*
