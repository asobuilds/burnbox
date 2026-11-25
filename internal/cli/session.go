package cli

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"syscall"
)

// workspaceDir is the RAM-workspace root used by burnbox-core.
const workspaceDir = "/dev/shm"

type workspace struct {
	PID  int
	Name string
	Path string
}

// scanWorkspaces returns the BurnBox workspaces under root, sorted by PID.
// The directory suffix is the orchestrator (burnbox-core) PID.
func scanWorkspaces(root string) ([]workspace, error) {
	entries, err := os.ReadDir(root)
	if err != nil {
		return nil, err
	}

	var found []workspace
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		name := entry.Name()
		if !strings.HasPrefix(name, "burnbox-") {
			continue
		}
		pid, err := strconv.Atoi(strings.TrimPrefix(name, "burnbox-"))
		if err != nil || pid <= 0 {
			continue
		}
		found = append(found, workspace{
			PID:  pid,
			Name: name,
			Path: filepath.Join(root, name),
		})
	}
	sort.Slice(found, func(i, j int) bool { return found[i].PID < found[j].PID })
	return found, nil
}

// processAlive reports whether a process still exists (permission counts as alive).
func processAlive(pid int) bool {
	err := syscall.Kill(pid, 0)
	return err == nil || errors.Is(err, syscall.EPERM)
}

// handleList implements `burnbox list`: show active RAM workspaces and PIDs.
func handleList(_ []string) error {
	sessions, err := scanWorkspaces(workspaceDir)
	if err != nil {
		// A missing /dev/shm simply means there is nothing to list.
		if os.IsNotExist(err) {
			fmt.Println("No active BurnBox sessions.")
			return nil
		}
		return fmt.Errorf("cannot list BurnBox sessions: %w", err)
	}
	if len(sessions) == 0 {
		fmt.Println("No active BurnBox sessions.")
		return nil
	}

	fmt.Println("Active BurnBox sessions:")
	for _, s := range sessions {
		state := "active"
		if !processAlive(s.PID) {
			state = "stale"
		}
		fmt.Printf("  PID %d  %s  [%s]\n", s.PID, s.Path, state)
	}
	return nil
}

// parseSessionID accepts a numeric PID, a workspace name (burnbox-1234), or a
// bare suffix (1234). Only positive, non-self PIDs are returned.
func parseSessionID(arg string, selfPID int) (int, error) {
	arg = strings.TrimSpace(arg)
	if arg == "" {
		return 0, fmt.Errorf("expected a PID or session name")
	}
	pidText := arg
	if !strings.HasPrefix(pidText, "burnbox-") {
		pidText = filepath.Base(pidText)
		if strings.HasPrefix(pidText, "burnbox-") {
			pidText = strings.TrimPrefix(pidText, "burnbox-")
		}
	} else {
		pidText = strings.TrimPrefix(pidText, "burnbox-")
	}

	pid, err := strconv.Atoi(pidText)
	if err != nil || pid <= 0 || pid == selfPID {
		return 0, fmt.Errorf("invalid session %q: expected PID or burnbox-<pid>", arg)
	}
	return pid, nil
}

// handleKill implements `burnbox kill <pid_or_name>`: SIGTERM the orchestrator
// so burnbox-core performs its signal-safe scrub and emits the wiped line.
func handleKill(args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("usage: burnbox kill <pid_or_name>")
	}
	pid, err := parseSessionID(args[0], os.Getpid())
	if err != nil {
		return err
	}

	sessions, err := scanWorkspaces(workspaceDir)
	if err != nil {
		return fmt.Errorf("cannot list BurnBox sessions: %w", err)
	}
	found := false
	for _, s := range sessions {
		if s.PID == pid {
			found = true
			break
		}
	}
	if !found {
		return fmt.Errorf("no active BurnBox session with PID %d (run 'burnbox list')", pid)
	}
	if !processAlive(pid) {
		return fmt.Errorf("BurnBox session PID %d is stale; workspace %s needs manual cleanup", pid, filepath.Join(workspaceDir, fmt.Sprintf("burnbox-%d", pid)))
	}

	proc, err := os.FindProcess(pid)
	if err != nil {
		return fmt.Errorf("cannot find orchestrator PID %d: %w", pid, err)
	}
	if err := proc.Signal(syscall.SIGTERM); err != nil {
		return fmt.Errorf("cannot signal orchestrator PID %d: %w", pid, err)
	}
	fmt.Printf("Sent SIGTERM to BurnBox orchestrator PID %d; safe scrub in progress.\n", pid)
	return nil
}
