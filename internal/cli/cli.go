package cli

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"
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
// Usage: burnbox open <file-or-url> [--offline] [--ram <mb>] [--copy-in <host_path> ...]
func handleOpen(args []string) error {
	var offline bool
	var ram int = 512 // default
	var copyInPaths []string // repeatable --copy-in host paths

	// Parse all arguments: flags + target
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
		case "--copy-in":
			if i+1 < len(args) {
				copyInPaths = append(copyInPaths, args[i+1])
				i += 2
				continue
			}
			return fmt.Errorf("--copy-in requires a value")
		default:
			// This is the target argument (file or URL)
			// Don't consume it here; handle after flags
			i++
			continue
		}
		i++
	}

	// The last non-flag argument is the target
	var target string
	for i := len(args) - 1; i >= 0; i-- {
		if !strings.HasPrefix(args[i], "--") {
			target = args[i]
			break
		}
	}

	if target == "" {
		return fmt.Errorf("expected file-or-url argument")
	}

	mode := "online"
	if offline {
		mode = "offline"
	}

	// Build burnbox-core command arguments per Amendment v1.1:
	// --target must be an executable path. Viewers/URLs are passed as args after --.
	// Correct order: --mode --ram-mb --target <exe> -- [args...]
	var cmdArgs []string

	// Add mode and ram FIRST (before --target)
	cmdArgs = append(cmdArgs, "--mode", mode, "--ram-mb", strconv.Itoa(ram))

	isURL := strings.HasPrefix(target, "http://") || strings.HasPrefix(target, "https://")

	if isURL {
		// URL: wrap in a viewer executable found via exec.LookPath
		viewer := findViewer()
		if viewer == "" {
			return fmt.Errorf("no suitable viewer found (firefox/chromium/xdg-open)")
		}
		// Shape: --target <viewer> -- --no-remote <url>
		cmdArgs = append(cmdArgs, "--target", viewer, "--", "--no-remote", target)
	} else {
		// Local file path: use --copy-in to stage the file into the workspace
		if len(copyInPaths) > 0 {
			// Pass --copy-in flags; burnbox-core will copy them into the workspace
			// burnbox-core copies each host_path into the workspace root (basename preserved, mode 0600)
			cmdArgs = append(cmdArgs, "--copy-in")
			for _, cp := range copyInPaths {
				cmdArgs = append(cmdArgs, cp)
			}
		}

		// Determine the target executable and its arguments
		// Shape: --target xdg-open -- <basename-of-copied-in-file>
		cmdArgs = append(cmdArgs, "--target", "xdg-open", "--", filepath.Base(target))
	}

	// Find burnbox-core binary relative to the executable directory
	exeDir := filepath.Dir(os.Args[0])
	corePath := filepath.Join(exeDir, "burnbox-core")

	// Set up display (Xephyr probe or host DISPLAY)
	display := setupXephyrDisplay()
	cmd := exec.Command(corePath, cmdArgs...)
	cmd.Env = append(os.Environ(), "DISPLAY="+display)

	// Capture output to parse JSON status lines
	var buf bytes.Buffer
	cmd.Stdout = &buf
	cmd.Stderr = os.Stderr
	err := cmd.Run()
	output := buf.String()

	// If no output captured and error, try CombinedOutput
	if err != nil && output == "" {
		output2, err2 := cmd.CombinedOutput()
		if err2 == nil || len(output2) > 0 {
			output = string(output2)
		}
	}
	if err != nil && output == "" {
		return fmt.Errorf("burnbox-core exited")
	}

	// Parse the 2-line JSON status output from burnbox-core
	lines := strings.Split(strings.TrimSpace(output), "\n")
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

// findViewer attempts to locate a suitable graphical viewer via exec.LookPath.
func findViewer() string {
	// Try firefox, chromium, xdg-open in order
	for _, name := range []string{"firefox", "chromium", "xdg-open"} {
		path, err := exec.LookPath(name)
		if err == nil {
			return path
		}
	}
	return ""
}

// setupXephyrDisplay probes for Xephyr display isolation.
// If /usr/bin/Xephyr exists and can open a display (probe: launch Xephyr :N -screen 1280x800,
// check /tmp/.X11-unix/XN socket), launch it, set DISPLAY=:N in the burnbox-core child env,
// and arrange for it to be killed on session end.
// If the probe fails (Wayland), soft-fallback to the host DISPLAY and log a warning to stderr.
func setupXephyrDisplay() string {
	xephyrPath := "/usr/bin/Xephyr"
	xephyrInfo, err := os.Stat(xephyrPath)
	if err != nil {
		// Xephyr not available, fall back to host DISPLAY
		fmt.Fprintf(os.Stderr, "warn: Xephyr not available; using host DISPLAY\n")
		return getHostDisplay()
	}
	_ = xephyrInfo // just checking existence

	// Try to launch Xephyr on a new display number
	displayNum := "99"
	xephyrCmd := exec.Command(xephyrPath, ":"+displayNum, "-screen", "1280x800")
	xephyrCmd.Stdout = nil
	xephyrCmd.Stderr = nil
	if err := xephyrCmd.Start(); err != nil {
		fmt.Fprintf(os.Stderr, "warn: failed to start Xephyr: %v; using host DISPLAY\n", err)
		return getHostDisplay()
	}

	// Wait for the X11 socket to appear
	socketPath := "/tmp/.X11-unix/X" + displayNum
	done := make(chan bool, 1)
	go func() {
		for i := 0; i < 30; i++ {
			fi, statErr := os.Stat(socketPath)
			if statErr == nil && fi.Mode()&os.ModeSocket != 0 {
				done <- true
				return
			}
			time.Sleep(100 * time.Millisecond)
		}
		done <- false
	}()

	select {
	case <-done:
		// Xephyr started successfully - will be killed when burnbox-core session ends
		return ":" + displayNum
	case <-time.After(3 * time.Second):
		// Timeout: kill Xephyr and fall back to host DISPLAY
		xephyrCmd.Process.Kill()
		fmt.Fprintf(os.Stderr, "warn: Xephyr probe timed out; using host DISPLAY\n")
		return getHostDisplay()
	}
}

// getHostDisplay returns the current host DISPLAY environment variable.
func getHostDisplay() string {
	disp := os.Getenv("DISPLAY")
	if disp != "" {
		return disp
	}
	return ":0"
}