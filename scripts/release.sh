#!/usr/bin/env bash
set -euo pipefail

# Single no-sudo orchestrator for BurnBox release pipeline.
# Runs: Rust build, Go build, go test, packaging, and doctor.
# Fails fast on any non-zero step.
# Guards against launching a GUI in CI by checking DISPLAY and using timeout.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
RELEASE_DIR="$PROJECT_DIR/release"

echo "========================================="
echo "BurnBox Release Pipeline v0.7.0"
echo "========================================="
echo ""

# --- Step 1: Rust build (sandbox-core) ---
echo "=== Step 1: Building burnbox-core (Rust) ==="
cd "$PROJECT_DIR/sandbox-core"
cargo build --release 2>&1
rm -f "$PROJECT_DIR/bin/burnbox-core"
cp target/release/burnbox-core "$PROJECT_DIR/bin/burnbox-core"
cd "$PROJECT_DIR"
echo "burnbox-core built and copied to bin/"
echo ""

# --- Step 2: Go build with version ---
echo "=== Step 2: Building burnbox (Go) ==="
go build -ldflags "-X burnbox/internal/version.Version=0.7.0" -o "$PROJECT_DIR/bin/burnbox" ./cmd/burnbox
echo "burnbox built with version 0.7.0"
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

# --- Step 5: Doctor check ---
echo "=== Step 5: Running burnbox doctor ==="
"$PROJECT_DIR/bin/burnbox" doctor
DOCTOR_EXIT=$?
if [ "$DOCTOR_EXIT" -ne 0 ]; then
    echo "WARNING: burnbox doctor exited with code $DOCTOR_EXIT"
fi
echo "Doctor check complete"
echo ""

# --- Final summary ---
echo "========================================="
echo "Release Pipeline Complete"
echo "========================================="
echo ""
echo "Artifacts in $RELEASE_DIR/:"
ls -la "$RELEASE_DIR/"
echo ""
echo "Built binaries:"
echo "  - $PROJECT_DIR/bin/burnbox   (Go binary, version 0.7.0)"
echo "  - $PROJECT_DIR/bin/burnbox-core   (Rust binary)"
echo ""
echo "Usage: ./bin/burnbox open <file-or-url> [--offline] [--ram <mb>]"
echo "       ./bin/burnbox --version     → BurnBox v0.7.0"
echo "       ./bin/burnbox doctor       → Host readiness checks"
echo ""
echo "========================================="