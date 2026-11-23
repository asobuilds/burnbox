package main

import (
	"fmt"
	"os"
	"os/signal"
	"syscall"

	"burnbox/internal/cli"
)

func main() {
	// Print friendly usage banner when no args or --help
	if len(os.Args) < 2 || os.Args[1] == "--help" || os.Args[1] == "-h" {
		fmt.Println("BurnBox - Unprivileged namespace sandbox and RAM wipe engine")
		fmt.Println()
		fmt.Println("Usage:")
		fmt.Println("  burnbox open <file-or-url> [--offline] [--ram <mb>] [--copy-in <host_path>...]")
		fmt.Println("  burnbox open <url> [--offline] [--ram <mb>]")
		fmt.Println()
		fmt.Println("Flags:")
		fmt.Println("  --offline, -o    Run in offline mode (no network namespace)")
		fmt.Println("  --ram <mb>       Size of the tmpfs RAM directory in mebibytes (default: 512)")
		fmt.Println("  --copy-in        Repeatable. Stage a host file into the sandbox workspace.")
		fmt.Println()
		fmt.Println("Try 'burnbox open <file>' to get started.")
		fmt.Println("💡 Tip: Run ./scripts/install.sh to add BurnBox to your 'Open With' context menu.")
		os.Exit(0)
	}

	// Setup signal handling for clean teardown
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, os.Interrupt, syscall.SIGTERM)

	// In a more complex app, we'd pass this channel down or use a context
	// For now, the CLI package will handle its own lifecycle, but we listen here too.
	go func() {
		<-sigChan
		// Handled by child process mostly, but ensure we exit
		os.Exit(0)
	}()

	if err := cli.Execute(); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
}
