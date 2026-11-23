package cli

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
)

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
	var fileOrURL string
	var offline bool
	var ram int = 512 // default

	// Simple argument parsing - distinguish flags from the file/URL argument
	var remainingArgs []string
	for _, arg := range args {
		switch arg {
		case "--offline", "-o":
			offline = true
		case "--ram", "-r":
			// will handle value below
		default:
			remainingArgs = append(remainingArgs, arg)
		}
	}

	// Parse --ram from remainingArgs if present
	// Re-parse to find --ram value
	i := 0
	for i < len(remainingArgs) {
		if remainingArgs[i] == "--ram" || remainingArgs[i] == "-r" {
			if i+1 < len(remainingArgs) {
				var err error
				 ram, err = strconv.Atoi(remainingArgs[i+1])
				 if err != nil {
					 return fmt.Errorf("--ram requires a valid integer: %s", remainingArgs[i+1])
				 }
				 i += 2
				 continue
			}
			return fmt.Errorf("--ram requires a value")
		}
		i++
	}

	// Now re-parse to extract file/URL and --ram properly
	fileOrURL = ""
	offline = false
	ram = 512
	i = 0
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
			if fileOrURL == "" {
				fileOrURL = args[i]
			} else {
				return fmt.Errorf("multiple file/URL arguments specified")
			}
		}
		i++
	}

	if fileOrURL == "" {
		return fmt.Errorf("expected file-or-url argument")
	}

	mode := "online"
	if offline {
		mode = "offline"
	}

	// Invoke burnbox-core per AGREEMENT.md syntax:
	// burnbox-core --mode <offline|online> --ram-mb <size> --target <executable_path> [--] [target_args...]
	// Find burnbox-core binary relative to the executable directory
	exePath := os.Args[0]
	exeDir := filepath.Dir(exePath)
	corePath := filepath.Join(exeDir, "burnbox-core")
	cmd := exec.Command(corePath, "--mode", mode, "--ram-mb", strconv.Itoa(ram), "--target", fileOrURL)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	cmd.Stdin = os.Stdin

	if err := cmd.Run(); err != nil {
		return fmt.Errorf("burnbox-core failed: %w", err)
	}

	return nil
}