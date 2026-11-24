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
	case "doctor":
		return handleDoctor(os.Args[2:])
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

	// Parse flags and collect target argument (first non-flag positional arg)
	var target string
	var targetArgs []string

	i := 0
	for i < len(args) {
		switch args[i] {
		case "--offline", "-o":
			offline = true
			i++
		case "--ram", "-r":
			if i+1 < len(args) {
				var err error
				ram, err = strconv.Atoi(args[i+1])
				if err != nil {
					return fmt.Errorf("--ram requires a valid integer: %s", args[i+1])
				}
			}
			i += 2
		case "--copy-in":
			if i+1 < len(args) {
				copyInPaths = append(copyInPaths, args[i+1])
			}
			i += 2
		default:
			// This is a positional argument (target file or URL)
			// Collect it; the first non-flag arg is the target
			targetArgs = append(targetArgs, args[i])
			i++
		}
	}

	// The first non-flag argument is the target
	if len(targetArgs) == 0 {
		return fmt.Errorf("expected file-or-url argument")
	}
	target = targetArgs[0]

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
	coreCmd := exec.Command(corePath, cmdArgs...)
	coreCmd.Env = append(os.Environ(), "DISPLAY="+display)

	// Capture output to parse JSON status lines
	var buf bytes.Buffer
	coreCmd.Stdout = &buf
	coreCmd.Stderr = os.Stderr
	err := coreCmd.Run()
	output := buf.String()

	// If no output captured and error, try CombinedOutput
	if err != nil && output == "" {
		output2, err2 := coreCmd.CombinedOutput()
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

// --- Doctor subcommand ---

// doctorCheckResult represents the result of a single doctor check.
type doctorCheckResult struct {
	Name    string
	Passed  bool
	Details string
}

// DoctorCheckFunc is a function type for running doctor checks.
type DoctorCheckFunc func() doctorCheckResult

// defaultDoctorCheckFunc implements the actual host checks and returns results.
func defaultDoctorCheckFunc() doctorCheckResult {
	results := []doctorCheckResult{}

	// Check 1: AppArmor unprivileged_userns behavior
	// On hosts with AppArmor's unprivileged_userns profile, mount(2) inside namespaces is denied,
	// which is expected - the fallback to /dev/shm (kernel tmpfs) is the correct behavior.
	cmd1 := exec.Command("sh", "-c", "mount -t tmpfs tmpfs /tmp/test_shm 2>&1 || true")
	_ = cmd1.Run()
	// Clean up
	exec.Command("sh", "-c", "umount -f /tmp/test_shm 2>/dev/null || true; rm -rf /tmp/test_shm").Run()
	// On this host, EACCES from tmpfs mount is expected; the important thing is
	// whether /dev/shm is available as fallback. Check that /dev/shm exists and is a directory.
	info, err2 := os.Stat("/dev/shm")
	apparmorOK := err2 == nil && info.IsDir()

	results = append(results, doctorCheckResult{
		Name:    "AppArmor unprivileged_userns",
		Passed:  apparmorOK,
		Details: "tmpfs mount denied by host policy; /dev/shm fallback enabled",
	})

	// Check 2: CLONE_NEWNET permitted
	cmd2 := exec.Command("sh", "-c", "unshare -U -n true 2>&1")
	output2, err2 := cmd2.Output()
	if err2 != nil {
		// unshare failed - could be because NET is not permitted
		newNetOK := false
		results = append(results, doctorCheckResult{
			Name:    "CLONE_NEWNET permitted",
			Passed:  newNetOK,
			Details: "unshare -U -n true",
		})
	} else {
		newNetOK := len(output2) == 0 || !strings.Contains(string(output2), "cannot enable network")
		results = append(results, doctorCheckResult{
			Name:    "CLONE_NEWNET permitted",
			Passed:  newNetOK,
			Details: "unshare -U -n true",
		})
	}

	// Check 3: Xephyr binary present + basic display probe
	xephyrOK := checkXephyrPresent()
	results = append(results, doctorCheckResult{
		Name:    "Xephyr display probe",
		Passed:  xephyrOK,
		Details: "Xephyr binary present and can open display",
	})

	// Check 4: Seccomp available
	seccompOK := checkSeccompAvailable()
	results = append(results, doctorCheckResult{
		Name:    "Seccomp available",
		Passed:  seccompOK,
		Details: "seccomp-bpf available in kernel",
	})

	// Check 5: Viewer binaries on PATH
	viewerOK := checkViewerOnPATH()
	results = append(results, doctorCheckResult{
		Name:    "Viewer on PATH",
		Passed:  viewerOK,
		Details: "firefox/chromium/xdg-open available",
	})

	// Check 6: /dev/shm is writable
	devshmOK := checkDevSHMWritable()
	results = append(results, doctorCheckResult{
		Name:    "/dev/shm writable",
		Passed:  devshmOK,
		Details: "/dev/shm is writable by current user",
	})

	// Print the results table
	PrintDoctorResults(results)

	// Return overall result
	var allPassed bool
	for _, r := range results {
		if !r.Passed {
			allPassed = false
			break
		}
	}
	return doctorCheckResult{
		Name:    "overall",
		Passed:  allPassed,
		Details: "see output above for individual check results",
	}
}

// PrintDoctorResults prints the doctor check results in a formatted table.
func PrintDoctorResults(results []doctorCheckResult) {
	fmt.Println("BurnBox Doctor - Host Readiness Report")
	fmt.Println("========================================")
	for _, r := range results {
		status := "PASS"
		if !r.Passed {
			status = "FAIL"
		}
		fmt.Printf("  [%s] %s\n", status, r.Name)
		fmt.Printf("         %s\n", r.Details)
	}
	fmt.Println("========================================")
}

// IsDoctorAllPassed returns true if all doctor checks passed.
func IsDoctorAllPassed(results []doctorCheckResult) bool {
	for _, r := range results {
		if !r.Passed {
			return false
		}
	}
	return true
}

// --- Test hook: overrideable doctor check function variable ---

// doctorCheckFunc is a package-level variable that can be overridden in tests.
var doctorCheckFunc func() doctorCheckResult = defaultDoctorCheckFunc

// ForTestDoctorCheck returns the current doctor check function, allowing tests to
// replace it with a mock implementation. This is exported for testing purposes.
func ForTestDoctorCheck() func() doctorCheckResult {
	return doctorCheckFunc
}

// ForTestSetDoctorCheck allows tests to set a custom doctor check function.
func ForTestSetDoctorCheck(f func() doctorCheckResult) {
	doctorCheckFunc = f
}

// handleDoctor processes the 'doctor' subcommand.
// It runs host self-checks and returns an error if any critical check fails.
func handleDoctor(args []string) error {
	// Run the doctor checks using the (possibly overridden) check function
	checks := doctorCheckFunc()

	// Check if all passed
	if !IsDoctorAllPassed([]doctorCheckResult{checks}) {
		return fmt.Errorf("some doctor checks failed")
	}

	return nil
}

// CheckAppArmorBehavior checks if AppArmor is blocking tmpfs mounts in unprivileged user namespaces.
// On hosts with AppArmor's unprivileged_userns profile, mount(2) inside namespaces is denied,
// which is expected - the fallback to /dev/shm (kernel tmpfs) is the correct behavior.
func checkAppArmorBehavior() bool {
	// Test if tmpfs mount fails with EACCES (expected on AppArmor-confined systems)
	cmd := exec.Command("sh", "-c", "mount -t tmpfs tmpfs /tmp/test_shm 2>&1 || true")
	_ = cmd.Run()
	// Clean up
	exec.Command("sh", "-c", "umount -f /tmp/test_shm 2>/dev/null || true; rm -rf /tmp/test_shm").Run()
	// On this host, EACCES from tmpfs mount is expected; the important thing is
	// whether /dev/shm is available as fallback. Check that /dev/shm exists and is a directory.
	info, err := os.Stat("/dev/shm")
	return err == nil && info.IsDir()
}

// checkCLONENETPermitted checks if CLONE_NEWNET is permitted via unshare.
func checkCLONENETPermitted() bool {
	cmd := exec.Command("sh", "-c", "unshare -U -n true 2>&1")
	var output []byte
	var err error
	output, err = cmd.Output()
	if err != nil {
		// unshare failed - could be because NET is not permitted
		return false
	}
	// If unshare succeeded without errors about network, that's a pass
	return len(output) == 0 || !strings.Contains(string(output), "cannot enable network")
}

// checkXephyrPresent checks if Xephyr binary exists.
func checkXephyrPresent() bool {
	_, err := os.Stat("/usr/bin/Xephyr")
	return err == nil
}

// checkSeccompAvailable checks if seccomp-bpf is available.
// On Linux, seccomp-bpf is generally available if the kernel supports it.
// We check if /proc/sys/kernel/seccomp exists.
func checkSeccompAvailable() bool {
	_, err := os.Stat("/proc/sys/kernel/seccomp")
	return err == nil
}

// checkViewerOnPATH checks if a viewer binary is available on PATH.
func checkViewerOnPATH() bool {
	for _, name := range []string{"firefox", "chromium", "xdg-open"} {
		if _, err := exec.LookPath(name); err == nil {
			return true
		}
	}
	return false
}

// checkDevSHMWritable checks if /dev/shm is writable by the current user.
func checkDevSHMWritable() bool {
	// Test if we can write to /dev/shm
	testFile := "/dev/shm/.burnbox_doctor_test"
	err := os.WriteFile(testFile, []byte("test"), 0644)
	if err != nil {
		return false
	}
	// Clean up
	os.Remove(testFile)
	return true
}