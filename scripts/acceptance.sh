#!/usr/bin/env bash
set -euo pipefail

# BurnBox End-to-End Acceptance Script
# Final user-facing validation of the burnbox open workflow.
# No sudo required.
#
# Tests:
#   1. Open a local dummy file offline — assert exit 0 and NO /dev/shm leak
#   2. Open a local HTML file online — assert exit 0
#   3. Open a live URL (via string concatenation) — assert success if slirp4netns
#      present, else graceful warning with clean exit

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BURNBOX_VERSION="0.8.0"

FAILURES=0
TOTAL_TESTS=0

echo "========================================="
echo "BurnBox Acceptance Test v${BURNBOX_VERSION}"
echo "========================================="
echo ""

# --- Pre-flight: clean up any prior /dev/shm leftovers ---
echo "=== Pre-flight: cleaning up /dev/shm/burnbox-* leftovers ==="
rm -rf /dev/shm/burnbox-* 2>/dev/null || true
SHM_BEFORE_ALL=$(ls -d /dev/shm/burnbox-* 2>/dev/null | wc -l || true)
echo "Found $SHM_BEFORE_ALL leftover burnbox-* directories in /dev/shm (should be 0)"
echo ""

# --- Verify binary exists ---
if [ ! -f "$PROJECT_DIR/bin/burnbox" ]; then
    echo "Error: bin/burnbox not found. Build it first." >&2
    exit 1
fi

BIN="$PROJECT_DIR/bin/burnbox"

# --- Test 1: Open a local dummy file offline ---
echo "=== Test 1: Open local file offline ==="
TOTAL_TESTS=$((TOTAL_TESTS + 1))

DUMMY_FILE="/tmp/burnbox_acceptance_dummy.txt"
echo "dummy acceptance test content for offline open" > "$DUMMY_FILE"

SHM_BEFORE_T1=$(ls -d /dev/shm/burnbox-* 2>/dev/null | wc -l || true)

set +e
"$BIN" open "$DUMMY_FILE" --offline --ram 512 --copy-in "$DUMMY_FILE" 2>&1
RC1=$?
set -e

SHM_AFTER_T1=$(ls -d /dev/shm/burnbox-* 2>/dev/null | wc -l || true)

if [ "$RC1" -ne 0 ]; then
    echo "❌ Test 1 FAILED: burnbox open (offline) exited with code $RC1"
    FAILURES=$((FAILURES + 1))
elif [ "$SHM_AFTER_T1" -gt "$SHM_BEFORE_T1" ]; then
    echo "❌ Test 1 FAILED: /dev/shm leak detected after offline test ($SHM_AFTER_T1 dirs vs $SHM_BEFORE_T1 before)"
    FAILURES=$((FAILURES + 1))
else
    echo "✅ Test 1 PASSED: offline open succeeded (rc=$RC1), no /dev/shm leak"
fi
echo ""

# --- Test 2: Open a local HTML file online ---
echo "=== Test 2: Open local HTML file online ==="
TOTAL_TESTS=$((TOTAL_TESTS + 1))

HTML_FILE="/tmp/burnbox_acceptance.html"
echo "<html><body><h1>Acceptance Test</h1></body></html>" > "$HTML_FILE"

SHM_BEFORE_T2=$(ls -d /dev/shm/burnbox-* 2>/dev/null | wc -l || true)

set +e
"$BIN" open "$HTML_FILE" --ram 512 --copy-in "$HTML_FILE" 2>&1
RC2=$?
set -e

SHM_AFTER_T2=$(ls -d /dev/shm/burnbox-* 2>/dev/null | wc -l || true)

if [ "$RC2" -ne 0 ]; then
    echo "❌ Test 2 FAILED: burnbox open (online, local file) exited with code $RC2"
    FAILURES=$((FAILURES + 1))
elif [ "$SHM_AFTER_T2" -gt "$SHM_BEFORE_T2" ]; then
    echo "❌ Test 2 FAILED: /dev/shm leak detected after online test"
    FAILURES=$((FAILURES + 1))
else
    echo "✅ Test 2 PASSED: online open succeeded (rc=$RC2)"
fi
echo ""

# --- Test 3: Open a live URL ---
echo "=== Test 3: Open live URL ==="
TOTAL_TESTS=$((TOTAL_TESTS + 1))

# Build URL via string concatenation (as required)
PROTO="https://"
HOST="example.com"
URL="${PROTO}${HOST}"

echo "Testing URL: $URL"

SHM_BEFORE_T3=$(ls -d /dev/shm/burnbox-* 2>/dev/null | wc -l || true)

# Check if slirp4netns is present
if command -v slirp4netns >/dev/null 2>&1; then
    echo "slirp4netns found — attempting URL open (expect success)"
    set +e
    "$BIN" open "$URL" --ram 512 2>&1
    RC3=$?
    set -e

    SHM_AFTER_T3=$(ls -d /dev/shm/burnbox-* 2>/dev/null | wc -l || true)

    if [ "$RC3" -eq 0 ] && [ "$SHM_AFTER_T3" -le "$SHM_BEFORE_T3" ]; then
        echo "✅ Test 3 PASSED: URL open succeeded with slirp4netns (rc=$RC3)"
    else
        echo "❌ Test 3 FAILED: URL open exited with code $RC3 or /dev/shm leak"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo "slirp4netns NOT found — expecting graceful warning (not a crash)"
    set +e
    "$BIN" open "$URL" --ram 512 2>&1
    RC3=$?
    set -e

    SHM_AFTER_T3=$(ls -d /dev/shm/burnbox-* 2>/dev/null | wc -l || true)

    # Accept rc=0 or rc=1 with clear message, not a crash
    if [ "$RC3" -eq 0 ] || [ "$RC3" -eq 1 ]; then
        if [ "$SHM_AFTER_T3" -le "$SHM_BEFORE_T3" ]; then
            echo "✅ Test 3 PASSED: URL open handled gracefully without slirp4netns (rc=$RC3)"
        else
            echo "❌ Test 3 FAILED: /dev/shm leak after graceful URL open failure"
            FAILURES=$((FAILURES + 1))
        fi
    else
        echo "❌ Test 3 FAILED: URL open crashed with unexpected exit code $RC3"
        FAILURES=$((FAILURES + 1))
    fi
fi
echo ""

# --- Cleanup ---
rm -f "$DUMMY_FILE" "$HTML_FILE"
rm -rf /dev/shm/burnbox-* 2>/dev/null || true

# --- Final Summary ---
echo "========================================="
echo "Acceptance Test Summary"
echo "========================================="
echo "Tests run: $TOTAL_TESTS"
echo "Failures:  $FAILURES"
echo ""

if [ "$FAILURES" -eq 0 ]; then
    echo "✅ ACCEPTANCE PASSED"
    exit 0
else
    echo "❌ ACCEPTANCE FAILED"
    exit 1
fi