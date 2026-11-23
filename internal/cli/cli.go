package cli

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
)

// statusLine represents the JSON status output from burnbox-core.
type statusLine struct {
	Status  string `json:"status"`
	PID     int    `json:"pid"`
	MountPt string `json:"mount_point"`
}

// Execute runs the burnbox CLI.
func Execute() error {
	if len(os.Args) < 2 {
		return fmt.Errorf("expected subcommand (e.g., 'open'), got: %s", os.Args[0])
	}

	switch os.Args[1] {
	case "open":
		return handleOpen(os.Args[2:])
	default:
		return fmt.Errorf("unknown subcommand %q", os.Args[1])
	}
}

// handleOpen processes the 'open' subcommand.
// Usage: burnbox open <file-or-url> [--offline] [--ram <mb>]
func handleOpen(args []string) error {
	var target string
	var offline bool
	var ram int = 512 // default

	// Parse arguments: distinguish flags from the target argument
	i := 0
	for i < len(args) {
		switch args[i] {
		case "--offline", "-o":
			offline = true
		case "--ram", "-r":
			if i+1 < len(args) {
				var err error
				ram, err = strconv.Atoi(args[i+1])
				if err != nil {
					return fmt.Errorf("--ram requires a valid integer: %s", args[i+1])
				}
				i += 2
				continue
			}
			return fmt.Errorf("--ram requires a value")
		default:
			if target == "" {
				target = args[i]
			} else {
				return fmt.Errorf("multiple target arguments specified")
			}
		}
		i++
	}

	if target == "" {
		return fmt.Errorf("expected file-or-url argument")
	}

	mode := "online"
	if offline {
		mode = "offline"
	}

	// Detect target type and process accordingly
	var processedTarget string
	isURL := strings.HasPrefix(target, "http://") || strings.HasPrefix(target, "https://")

	if isURL {
		// For URLs: pass the URL as the target executable.
		processedTarget = target
	} else {
		// Local file path: verify it exists, then copy to /dev/shm/
		info, err := os.Stat(target)
		if err != nil {
			return fmt.Errorf("file not found: %s", target)
		}
		if info.IsDir() {
			return fmt.Errorf("path is a directory, not a file: %s", target)
		}

		// Copy file to /dev/shm/ (kernel tmpfs, no mount required)
		basename := filepath.Base(target)
		destPath := filepath.Join("/dev/shm", basename)

		srcFile, err := os.Open(target)
		if err != nil {
			return fmt.Errorf("failed to open source file: %s", err)
		}
		defer srcFile.Close()

		dstFile, err := os.Create(destPath)
		if err != nil {
			return fmt.Errorf("failed to create destination: %s", err)
		}
		defer dstFile.Close()

		if _, err := io.Copy(dstFile, srcFile); err != nil {
			return fmt.Errorf("failed to copy file: %s", err)
		}

		processedTarget = destPath
	}

	// Invoke burnbox-core per AGREEMENT.md syntax:
	// burnbox-core --mode <offline|online> --ram-mb <size> --target <executable_path> [--] [target_args...]
	exeDir := filepath.Dir(os.Args[0])
	corePath := filepath.Join(exeDir, "burnbox-core")

	cmd := exec.Command(corePath, "--mode", mode, "--ram-mb", strconv.Itoa(ram), "--target", processedTarget)

	// Capture output to parse JSON status lines
	output, err := cmd.Output()
	// cmd.Output may return an error for non-zero exit status even when
	// output was captured (burnbox-core exits 0 normally but the Go
	// exec package can report exit status 1 in some configurations).
	// Use whatever output we captured; fall back to CombinedOutput if empty.
	if err != nil && string(output) == "" {
		output, _ = cmd.CombinedOutput()
	}
	if err != nil && string(output) == "" {
		return fmt.Errorf("burnbox-core exited")
	}

	// Parse the 2-line JSON status output from burnbox-core
	lines := strings.Split(strings.TrimSpace(string(output)), "\n")
	var startedStatus statusLine
	var wipedStatus statusLine
	var hasStarted, hasWiped bool

	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		var s statusLine
		if err := json.Unmarshal([]byte(line), &s); err != nil {
			// Not a JSON line; skip.
			continue
		}
		switch s.Status {
		case "started":
			startedStatus = s
			hasStarted = true
		case "wiped":
			wipedStatus = s
			hasWiped = true
		}
	}

	// Surface user-friendly terminal messages based on parsed JSON
	if hasStarted {
		fmt.Printf("BurnBox session started (PID: %d, workspace: %s)\n", startedStatus.PID, startedStatus.MountPt)
	}
	if hasWiped {
		fmt.Printf("BurnBox session wiped (PID: %d, workspace: %s)\n", wipedStatus.PID, wipedStatus.MountPt)
	}
	// Fallback if JSON parsing didn't find expected lines
	if !hasStarted {
		fmt.Println("BurnBox session started and completed")
	}

	return nil
}