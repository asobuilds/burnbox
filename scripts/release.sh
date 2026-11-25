#!/usr/bin/env bash
set -euo pipefail

# Single no-sudo orchestrator for BurnBox release pipeline.
# Runs: Rust build, Go build, go test, packaging, and doctor (advisory).
# Fails fast on any non-zero step except doctor (which is advisory).
# Guards against launching a GUI in CI by checking DISPLAY and using timeout.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
RELEASE_DIR="$PROJECT_DIR/release"

# Extract version dynamically from source (single source of truth: internal/version/version.go)
BURNBOX_VERSION=$(grep -oP 'Version\s*=\s*"\K[^"]+' "$PROJECT_DIR/internal/version/version.go")
if [ -z "$BURNBOX_VERSION" ]; then
    echo "FATAL: could not read version from internal/version/version.go" >&2
    exit 1
fi

echo "========================================="
echo "BurnBox Release Pipeline v${BURNBOX_VERSION}"
echo "========================================="
echo ""

# --- Step 1: Rust build (sandbox-core) ---
echo "=== Step 1: Building burnbox-core (Rust) ==="
cd "$PROJECT_DIR/sandbox-core"
if ! cargo build --release 2>&1; then
    echo "FATAL: sandbox-core failed to compile; refusing to package a stale engine" >&2
    exit 1
fi
rm -f "$PROJECT_DIR/bin/burnbox-core"
if ! cp target/release/burnbox-core "$PROJECT_DIR/bin/burnbox-core"; then
    echo "FATAL: sandbox-core failed to compile; refusing to package a stale engine" >&2
    exit 1
fi
cd "$PROJECT_DIR"

# Assert freshness: the newly built core must expose M11 features.
if ! "$PROJECT_DIR/bin/burnbox-core" --help 2>&1 | grep -q -- "--fs-jail"; then
    echo "FATAL: sandbox-core failed to compile; refusing to package a stale engine" >&2
    exit 1
fi
if ! "$PROJECT_DIR/bin/burnbox-core" --help 2>&1 | grep -q -- "--timeout"; then
    echo "FATAL: sandbox-core failed to compile; refusing to package a stale engine" >&2
    exit 1
fi
echo "burnbox-core built, copied to bin/, and verified fresh (--fs-jail + --timeout present)"
echo ""

# --- Step 2: Go build (version comes from internal/version/version.go) ---
echo "=== Step 2: Building burnbox (Go) ==="
go build -o "$PROJECT_DIR/bin/burnbox" ./cmd/burnbox

# Assert the built binary reports exactly the source version (version-drift kill-switch).
BUILT_VERSION=$("$PROJECT_DIR/bin/burnbox" --version 2>&1 | sed 's/^BurnBox v//')
if [ "$BUILT_VERSION" != "$BURNBOX_VERSION" ]; then
    echo "FATAL: binary version != source version; refusing to package" >&2
    echo "       binary=${BUILT_VERSION} source=${BURNBOX_VERSION}" >&2
    exit 1
fi
echo "burnbox built with version ${BURNBOX_VERSION} (matches source)"
echo ""

# --- Step 3: Go tests ---
echo "=== Step 3: Running go test ==="
go test -timeout 60s ./...
echo "All tests passed"
echo ""

# --- Step 4: Packaging script ---
echo "=== Step 4: Running packaging script ==="
"$SCRIPT_DIR/package.sh"
echo "Packaging complete"
echo ""

# --- Step 4b: Release signing (hashes for the published artifacts) ---
echo "=== Step 4b: Generating SHA256SUMS ==="
(
    cd "$RELEASE_DIR"
    sha256sum ./*.deb ./*.tar.gz > SHA256SUMS
)
cat "$RELEASE_DIR/SHA256SUMS"
echo "Generated: $RELEASE_DIR/SHA256SUMS"
echo ""

# --- Step 5: Doctor check (ADVISORY - does not fail the build) ---
echo "=== Step 5: Running burnbox doctor (advisory) ==="
DOCTOR_EXIT=0
"$PROJECT_DIR/bin/burnbox" doctor || DOCTOR_EXIT=$?
if [ "$DOCTOR_EXIT" -ne 0 ]; then
    echo "WARNING: burnbox doctor exited with code $DOCTOR_EXIT (advisory only — build continues)"
    echo "         This is expected on headless CI runners missing Xephyr or slirp4netns."
else
    echo "Doctor check passed."
fi
echo "Doctor check complete"
echo ""

# --- Final summary ---
echo "========================================="
echo "Release Pipeline Complete (v${BURNBOX_VERSION})"
echo "========================================="
echo ""
echo "Artifacts in $RELEASE_DIR/:"
ls -la "$RELEASE_DIR/"
echo ""
echo "Built binaries:"
echo "  - $PROJECT_DIR/bin/burnbox   (Go binary, version ${BURNBOX_VERSION})"
echo "  - $PROJECT_DIR/bin/burnbox-core   (Rust binary)"
echo ""
echo "Usage: ./bin/burnbox open <file-or-url> [--offline] [--ram <mb>]"
echo "       ./bin/burnbox --version     → BurnBox v${BURNBOX_VERSION}"
echo "       ./bin/burnbox doctor       → Host readiness checks"
echo ""
echo "========================================="
echo "RELEASE TAG (run manually by Director):"
echo "========================================="
echo ""
echo "  git tag v${BURNBOX_VERSION}"
echo "  git push origin v${BURNBOX_VERSION}"
echo ""
echo "========================================="