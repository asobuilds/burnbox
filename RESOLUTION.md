# Conflict & Error Resolution Protocol

## 1. Concurrency & Git Rules
- **Atomic Commits:** Each agent must commit working changes to its own directory immediately after passing tests (`git add <allowed_path> && git commit -m "<agent>: <summary>"`).
- **No Force Pushing / No Global Resets:** Never run `git reset --hard` or `git clean -fd` without explicit user approval, as it will destroy the concurrent agent's uncommitted work.
- **Lock Contention:** If an agent needs to edit `AGREEMENT.md` or `RESOLUTION.md` and detects uncommitted changes in `git status`, wait 10 seconds, re-read the file, and append cleanly.

## 2. Error Escalation & Handling
- **Compilation Breakage:** If a change breaks the build in your domain, fix it immediately before touching new features.
- **Cross-Domain Mismatch:** If `burnbox` (Go) fails to parse output from `burnbox-core` (Rust), log the exact stdout/stderr under **Active Incident Log** below rather than editing the other agent's codebase.

## 3. Active Incident Log

### 2026-11-23 — Host AppArmor blocks tmpfs mount in unprivileged user namespaces (Agent A)
- **Symptom:** On this host, `burnbox-core` creates `CLONE_NEWUSER|CLONE_NEWNS|CLONE_NEWPID` successfully and the parent writes `uid_map`/`gid_map` correctly (child sees uid 0 with a full capability set), but `mount("tmpfs", ...)` returns `EACCES`.
- **Root cause:** The kernel automatically confines new user namespaces with the Ubuntu AppArmor profile `unprivileged_userns (enforce)` (`/proc/self/attr/current` in the child), which denies mount operations. Host user `student` has no sudo/AppArmor admin rights, so the profile cannot be relaxed here.
- **Impact:** End-to-end runtime verification of tmpfs mounting, `/home` hiding and target execution cannot be completed on this host. The binary fails closed with exit code `2` and still emits the exact `started`/`wiped` JSON lines.
- **Status:** Implementation follows the documented unprivileged-namespace flow (namespace creator writes maps, sync pipe, `CAP_SYS_ADMIN` in the new userns) and is expected to run on hosts that permit unprivileged userns mounts. No code change can bypass a host LSM denial without violating the no-sudo constraint. No cross-domain code was modified.
