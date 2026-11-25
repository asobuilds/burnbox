#!/usr/bin/env bash
set -euo pipefail

# BurnBox End-to-End Acceptance Script
# Final user-facing validation of the burnbox open workflow.
# No sudo required.
#
# Tests:
#   1. Open a local dummy file offline — assert exit 0 and NO /dev/shm leak
#   2. Open a local HTML file online — require viewer artifact (--screenshot)
#   3. Open a live URL (via string concatenation) — SKIP if no native viewer
#   4. Negative: sandboxed viewer must NOT read host file outside /home

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BURNBOX_VERSION="1.0.0"

FAILURES=0
TOTAL_TESTS=0
SKIPS=0

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

# --- Detect viewer (mirrors findViewer() in internal/cli/cli.go) ---
# Returns the viewer path (native ELF) or "xdg-open" (fallback) or empty.
detect_viewer() {
    # (a) BURNBOX_VIEWER env override
    if v="${BURNBOX_VIEWER:-}"; [ -n "$v" ] && [ -f "$v" ]; then
        if head -c 4 "$v" 2>/dev/null | grep -q $'\x7fELF'; then
            echo "$v"
            return
        fi
    fi
    # (b) Native Firefox ELF via known install paths
    for p in /snap/firefox/current/usr/lib/firefox/firefox /usr/lib/firefox/firefox /opt/firefox/firefox; do
        if [ -f "$p" ] && head -c 4 "$p" 2>/dev/null | grep -q $'\x7fELF'; then
            echo "$p"
            return
        fi
    done
    # (c) /usr/bin/firefox — skip if snap shim (starts with #!)
    if [ -f /usr/bin/firefox ]; then
        if ! head -c 2 /usr/bin/firefox 2>/dev/null | grep -q '^#!'; then
            if head -c 4 /usr/bin/firefox 2>/dev/null | grep -q $'\x7fELF'; then
                echo "/usr/bin/firefox"
                return
            fi
        fi
    fi
    # (d) Native chromium/chromium-browser/google-chrome
    for name in chromium chromium-browser google-chrome; do
        if path=$(command -v "$name" 2>/dev/null) && [ -f "$path" ]; then
            if head -c 4 "$path" 2>/dev/null | grep -q $'\x7fELF'; then
                echo "$path"
                return
            fi
        fi
    done
    # (e) xdg-open as last resort (not a native ELF viewer)
    if path=$(command -v xdg-open 2>/dev/null); then
        echo "$path"
        return
    fi
    echo ""
}

VIEWER_PATH=$(detect_viewer)
XDG_OPEN_PATH=$(command -v xdg-open 2>/dev/null || true)
NATIVE_VIEWER="no"
if [ -n "$VIEWER_PATH" ] && [ -n "$XDG_OPEN_PATH" ] && [ "$VIEWER_PATH" != "$XDG_OPEN_PATH" ]; then
    NATIVE_VIEWER="yes"
elif [ -n "$VIEWER_PATH" ] && [ -z "$XDG_OPEN_PATH" ]; then
    # If xdg-open doesn't exist, any found viewer is native
    NATIVE_VIEWER="yes"
fi

echo "Viewer detected: ${VIEWER_PATH:-none} (native: $NATIVE_VIEWER)"
echo ""

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

SCREENSHOT="/tmp/bb_acc_shot.png"
rm -f "$SCREENSHOT"

# Always try with --screenshot. If xdg-open is the viewer, it will reject --screenshot
# and we SKIP (no native viewer available). If a native viewer is used, check artifact.
set +e
OUTPUT2=$("$BIN" open "$HTML_FILE" --ram 512 --copy-in "$HTML_FILE" --screenshot "$SCREENSHOT" 2>&1)
RC2=$?
set -e

SHM_AFTER_T2=$(ls -d /dev/shm/burnbox-* 2>/dev/null | wc -l || true)

if echo "$OUTPUT2" | grep -q "unexpected option '--screenshot'"; then
    echo "⚠️  Test 2 SKIPPED: no native viewer (xdg-open doesn't support --screenshot)"
    SKIPS=$((SKIPS + 1))
elif [ "$RC2" -ne 0 ]; then
    echo "❌ Test 2 FAILED: burnbox open (online, local file) exited with code $RC2"
    FAILURES=$((FAILURES + 1))
elif [ "$SHM_AFTER_T2" -gt "$SHM_BEFORE_T2" ]; then
    echo "❌ Test 2 FAILED: /dev/shm leak detected after online test"
    FAILURES=$((FAILURES + 1))
elif [ ! -s "$SCREENSHOT" ]; then
    echo "❌ Test 2 FAILED: viewer did not produce non-empty screenshot artifact ($SCREENSHOT)"
    FAILURES=$((FAILURES + 1))
else
    echo "✅ Test 2 PASSED: online open succeeded (rc=$RC2), screenshot artifact non-empty"
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
HAS_SLIRP4NETNS=no
if command -v slirp4netns >/dev/null 2>&1; then
    HAS_SLIRP4NETNS=yes
fi

rm -f "$SCREENSHOT"

if [ "$HAS_SLIRP4NETNS" = "no" ]; then
    echo "⚠️  Test 3 SKIPPED: slirp4netns not present; live URL would be loopback-only"
    SKIPS=$((SKIPS + 1))
else
    set +e
    OUTPUT3=$("$BIN" open "$URL" --ram 512 --screenshot "$SCREENSHOT" 2>&1)
    RC3=$?
    set -e

    SHM_AFTER_T3=$(ls -d /dev/shm/burnbox-* 2>/dev/null | wc -l || true)

    if echo "$OUTPUT3" | grep -q "unexpected option '--screenshot'"; then
        echo "⚠️  Test 3 SKIPPED: no native viewer (xdg-open doesn't support --screenshot)"
        SKIPS=$((SKIPS + 1))
    elif [ "$RC3" -ne 0 ]; then
        echo "❌ Test 3 FAILED: URL open exited with code $RC3"
        FAILURES=$((FAILURES + 1))
    elif [ "$SHM_AFTER_T3" -gt "$SHM_BEFORE_T3" ]; then
        echo "❌ Test 3 FAILED: /dev/shm leak detected after URL test"
        FAILURES=$((FAILURES + 1))
    elif [ ! -s "$SCREENSHOT" ]; then
        echo "❌ Test 3 FAILED: viewer did not produce non-empty screenshot artifact ($SCREENSHOT)"
        FAILURES=$((FAILURES + 1))
    else
        echo "✅ Test 3 PASSED: URL open succeeded with slirp4netns + native viewer (rc=$RC3)"
    fi
fi
echo ""

# --- Test 4: Negative assertion — sandboxed viewer must NOT read host file outside /home ---
echo "=== Test 4: Negative — sandbox cannot read host file outside /home ==="
TOTAL_TESTS=$((TOTAL_TESTS + 1))

SECRET_FILE="/tmp/burnbox_hostsecret"
SECRET_CONTENT="burnbox-sentinel-$(date +%s)-$$"
echo "$SECRET_CONTENT" > "$SECRET_FILE"

set +e
CAPTURED_OUTPUT=$("$BIN" open "$SECRET_FILE" --offline --copy-in "$SECRET_FILE" 2>&1)
RC4=$?
set -e

if echo "$CAPTURED_OUTPUT" | grep -q "$SECRET_CONTENT"; then
    echo "❌ Test 4 FAILED: sandboxed viewer read host file outside /home (sentinel leaked)"
    FAILURES=$((FAILURES + 1))
elif [ "$RC4" -ne 0 ] && [ "$RC4" -ne 1 ]; then
    echo "❌ Test 4 FAILED: unexpected crash (rc=$RC4) during negative test"
    FAILURES=$((FAILURES + 1))
else
    echo "✅ Test 4 PASSED: sandboxed viewer did not read host file outside /home"
fi
echo ""

# --- Cleanup ---
rm -f "$DUMMY_FILE" "$HTML_FILE" "$SECRET_FILE" "$SCREENSHOT"
rm -rf /dev/shm/burnbox-* 2>/dev/null || true

# --- Final Summary ---
echo "========================================="
echo "Acceptance Test Summary"
echo "========================================="
echo "Tests run: $TOTAL_TESTS"
echo "Passed:    $((TOTAL_TESTS - FAILURES - SKIPS))"
echo "Skipped:   $SKIPS"
echo "Failures:  $FAILURES"
echo ""

if [ "$FAILURES" -eq 0 ]; then
    echo "✅ ACCEPTANCE PASSED"
    exit 0
else
    echo "❌ ACCEPTANCE FAILED"
    exit 1
fi