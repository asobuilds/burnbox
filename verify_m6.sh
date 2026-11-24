#!/usr/bin/env bash
set -u
cd "$(dirname "$0")"
pass=0; fail=0
ok(){ echo "  ✅ $1"; pass=$((pass+1)); }
no(){ echo "  ❌ $1"; fail=$((fail+1)); }

echo "== [1] version 0.6.0 =="
out="$(./bin/burnbox --version 2>&1)"; rc=$?
echo "  got: $out (rc=$rc)"
[ $rc -eq 0 ] && echo "$out" | grep -q '0.6.0' && ok "version 0.6.0" || no "version"

echo "== [2] SIGTERM mid-session => workspace scrubbed (THE fix) =="
rm -rf /dev/shm/burnbox-* 2>/dev/null
./bin/burnbox-core --mode online --ram-mb 512 --target /bin/sleep -- 30 >/tmp/bb2.json 2>/dev/null &
cpid=$!
sleep 1
if [ -d "$(ls -d /dev/shm/burnbox-* 2>/dev/null | head -n1)" ]; then
  ok "workspace existed pre-kill"
else
  no "workspace never created (test invalid)"
fi
kill -TERM $cpid 2>/dev/null
wait $cpid 2>/dev/null
sleep 1
if ls /dev/shm/ 2>/dev/null | grep -qi burnbox; then
  no "SIGTERM LEAKED /dev/shm/burnbox-*"
else
  ok "SIGTERM scrubbed workspace — no leak"
fi

echo "== [3] strict allow-list: normal sh must NOT be killed =="
./bin/burnbox-core --mode online --ram-mb 512 --seccomp-strict --target /bin/sh -- -c 'echo hi' >/tmp/bb3.txt 2>/tmp/bb3.err; rc=$?
echo "  rc=$rc (expect 0)"
[ $rc -eq 0 ] && grep -q hi /tmp/bb3.txt && ok "strict sh ok" || no "strict sh rc=$rc"
grep -qi 'strict seccomp' /tmp/bb3.err && ok "strict warning on stderr" || no "missing strict warning"

echo "== [4] strict allow-list: ptrace MUST be killed -> 3 =="
PY="$(command -v python3 || true)"
if [ -n "$PY" ]; then
  ./bin/burnbox-core --mode online --ram-mb 512 --seccomp-strict --target "$PY" -- -c 'import ctypes; ctypes.CDLL("libc.so.6").ptrace(0,0,0,0)' >/dev/null 2>/dev/null; rc=$?
  echo "  rc=$rc (expect 3)"
  [ $rc -eq 3 ] && ok "strict ptrace killed -> 3" || no "strict ptrace rc=$rc (allow-list too permissive?)"
else
  echo "  ⚠️ python3 missing; skipped"
fi

echo "== [5] burnbox doctor => PASS table, exit 0 =="
./bin/burnbox doctor >/tmp/bb5.txt 2>&1; rc=$?
echo "  rc=$rc (expect 0)"
grep -qiE 'PASS|OK|✓' /tmp/bb5.txt && ok "doctor printed checks" || no "doctor no table"
[ $rc -eq 0 ] && ok "doctor exit 0" || no "doctor rc=$rc"

echo "== [6] default deny-list still live (regression) =="
if [ -n "$PY" ]; then
  ./bin/burnbox-core --mode online --ram-mb 512 --target "$PY" -- -c 'import ctypes; ctypes.CDLL("libc.so.6").ptrace(0,0,0,0)' >/dev/null 2>/dev/null; rc=$?
  [ $rc -eq 3 ] && ok "default ptrace -> 3" || no "default ptrace rc=$rc"
fi

echo; echo "==== VERDICT: $pass ok / $fail fail ===="
