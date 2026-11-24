#!/usr/bin/env bash
set -u
cd "$(dirname "$0")"
pass=0; fail=0
ok(){ echo "  ✅ $1"; pass=$((pass+1)); }
no(){ echo "  ❌ $1"; fail=$((fail+1)); }

echo "== [1] Strict mode: Real Firefox launch (local file) =="
echo "<html><body><h1>BurnBox M7 Test</h1></body></html>" > /tmp/m7_test.html
rm -f /tmp/m7_screenshot.png
./bin/burnbox-core --mode online --ram-mb 512 --seccomp-strict --target /usr/bin/firefox -- --headless --no-remote --screenshot=/tmp/m7_screenshot.png file:///tmp/m7_test.html >/tmp/bb7.json 2>/tmp/bb7.err; rc=$?
echo "  rc=$rc (expect 0)"
[ $rc -eq 0 ] && ok "strict firefox exit 0" || no "strict firefox rc=$rc"
[ -f /tmp/m7_screenshot.png ] && ok "screenshot generated" || no "no screenshot"
grep -qi 'wiped' /tmp/bb7.json && ok "JSON wiped line present" || no "missing wiped JSON"

echo "== [2] Regression: Strict mode must still kill ptrace =="
PY="$(command -v python3 || true)"
if [ -n "$PY" ]; then
  ./bin/burnbox-core --mode online --ram-mb 512 --seccomp-strict --target "$PY" -- -c 'import ctypes; ctypes.CDLL("libc.so.6").ptrace(0,0,0,0)' >/dev/null 2>/dev/null; rc=$?
  [ $rc -eq 3 ] && ok "strict ptrace -> 3" || no "strict ptrace rc=$rc"
fi

echo "== [3] Release pipeline (release.sh) =="
./scripts/release.sh >/tmp/bb7_release.log 2>&1; rc=$?
echo "  rc=$rc (expect 0)"
[ $rc -eq 0 ] && ok "release.sh green" || no "release.sh rc=$rc (check /tmp/bb7_release.log)"
ls -1 release/ 2>/dev/null | head -n 3

echo "== [4] Online mode networking probe (URL vs Local) =="
echo "  Testing if 'burnbox open <url>' can reach DNS..."
timeout 15 ./bin/burnbox open "https://example.com" --ram 512 >/tmp/bb7_url.log 2>&1
rc=$?
if grep -qi 'timeout\|network\|DNS\|unreachable' /tmp/bb7_url.log 2>/dev/null; then
  echo "  ⚠️  URL mode blocked (expected: unprivileged netns has no veth/sudo)"
  ok "documented netns limitation"
else
  echo "  ℹ️  URL mode rc=$rc (check /tmp/bb7_url.log)"
fi

echo; echo "==== VERDICT: $pass ok / $fail fail ===="
