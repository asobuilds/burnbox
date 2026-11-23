# Conflict & Error Resolution Protocol

## 1. Concurrency & Git Rules
- **Atomic Commits:** Each agent must commit working changes to its own directory immediately after passing tests (`git add <allowed_path> && git commit -m "<agent>: <summary>"`).
- **No Force Pushing / No Global Resets:** Never run `git reset --hard` or `git clean -fd` without explicit user approval, as it will destroy the concurrent agent's uncommitted work.
- **Lock Contention:** If an agent needs to edit `AGREEMENT.md` or `RESOLUTION.md` and detects uncommitted changes in `git status`, wait 10 seconds, re-read the file, and append cleanly.

## 2. Error Escalation & Handling
- **Compilation Breakage:** If a change breaks the build in your domain, fix it immediately before touching new features.
- **Cross-Domain Mismatch:** If `burnbox` (Go) fails to parse output from `burnbox-core` (Rust), log the exact stdout/stderr under **Active Incident Log** below rather than editing the other agent's codebase.

## 3. Active Incident Log
*No active incidents.*
