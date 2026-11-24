package version

// Version is the current BurnBox version set at build time via ldflags.
// Example: go build -ldflags "-X burnbox/internal/version.Version=1.0.0"
var Version = "1.0.0"

// PrintVersion returns the formatted version string.
func PrintVersion() string {
	return "BurnBox v" + Version
}