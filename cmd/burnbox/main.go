package main

import (
	"fmt"
	"os"
	"os/signal"
	"syscall"

	"burnbox/internal/cli"
	"burnbox/internal/version"
)

// Print friendly usage banner when no args or --help
func printUsage() {
	fmt.Println("BurnBox - Unprivileged namespace sandbox and RAM wipe engine")
	fmt.Println()
	fmt.Println("Usage:")
	fmt.Println("  burnbox open <file-or-url> [--offline] [--ram <mb>] [--copy-in <host_path>...]")
	fmt.Println("  burnbox open <url> [--offline] [--ram <mb>]")
	fmt.Println("  burnbox doctor   Run host readiness checks")
	fmt.Println("  burnbox list     List active RAM workspaces and their PIDs")
	fmt.Println("  burnbox kill <pid_or_name>   SIGTERM a session to trigger the safe scrub")
	fmt.Println()
	fmt.Println("Flags:")
	fmt.Println("  --offline, -o    Run in offline mode (no network namespace)")
	fmt.Println("  --online         Force online mode (overrides an offline config default)")
	fmt.Println("  --ram <mb>       Size of the tmpfs RAM directory in mebibytes (default: 512)")
	fmt.Println("  --copy-in        Repeatable. Stage a host file into the sandbox workspace.")
	fmt.Println("  --screenshot <path>  Request a screenshot artifact from the viewer")
	fmt.Println()
	fmt.Println("Commands:")
	fmt.Println("  open <file-or-url>    Open a file or URL in the sandbox")
	fmt.Println("  doctor               Run host readiness checks")
	fmt.Println("  list                 List active BurnBox sessions")
	fmt.Println("  kill <pid_or_name>   Stop a BurnBox session safely")
	fmt.Println()
	fmt.Println("Config: ~/.config/burnbox/config.toml (or config.json) sets default")
	fmt.Println("        mode and ram_mb; CLI flags override them.")
	fmt.Println()
	fmt.Println("💡 Tip: Run ./scripts/install.sh to add BurnBox to your 'Open With' context menu.")
	fmt.Println("💡 Tip: Run burnbox doctor to verify your host setup.")
}

// Print version and exit
func printVersion() {
	fmt.Printf("%s\n", version.PrintVersion())
	os.Exit(0)
}

// printDoctorBanner prints the doctor help text
func printDoctorBanner() {
	fmt.Println("burnbox doctor - Host Readiness Checks")
	fmt.Println("Run 'burnbox doctor' to check your host's suitability for BurnBox sandboxing.")
	fmt.Println("This checks AppArmor, namespace capabilities, display servers, and more.")
	fmt.Println()
}

// doctorMain runs the doctor subcommand via cli.Execute() and prints a summary.
func doctorMain() int {
	// Print the doctor banner
	printDoctorBanner()

	// Run doctor checks via cli.Execute() -> handleDoctor() -> doctorCheckFunc()
	if err := cli.Execute(); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		return 1
	}

	// Print a summary line
	fmt.Println("========================================")
	fmt.Println("BurnBox Doctor complete.")
	fmt.Println("Review the output above for any FAIL entries.")
	fmt.Println("========================================")

	return 0
}

func main() {
	// Print version and exit
	if len(os.Args) > 1 && (os.Args[1] == "--version" || os.Args[1] == "-v") {
		printVersion()
	}

	// Print friendly usage banner when no args or --help
	if len(os.Args) < 2 || os.Args[1] == "--help" || os.Args[1] == "-h" {
		printUsage()
		os.Exit(0)
	}

	// Handle the doctor subcommand
	if os.Args[1] == "doctor" {
		exitCode := doctorMain()
		os.Exit(exitCode)
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
