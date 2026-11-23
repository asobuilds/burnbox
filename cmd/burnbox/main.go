package main

import (
	"fmt"
	"os"
	"os/signal"
	"syscall"

	"burnbox/internal/cli"
)

func main() {
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
