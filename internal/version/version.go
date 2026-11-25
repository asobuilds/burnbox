package version

// Version is the single source of truth for the BurnBox release version.
// Do not override it with ldflags: scripts/release.sh derives the release
// version from this value and verifies the built binary matches it.
var Version = "1.0.1"

// PrintVersion returns the formatted version string.
func PrintVersion() string {
	return "BurnBox v" + Version
}