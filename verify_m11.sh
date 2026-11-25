#!/usr/bin/env bash
set -u
cd "$(dirname "$0")"
pass=0; fail=0; skip=0
ok(){ echo "  ✅ $1"; pass=$((pass+1)); }
no(){ echo "  ❌ $1"; fail=$((fail+1)); }
sk(){ echo "  ⏭️  $1"; skip=$((skip+1)); }

echo "== [1] Clean-room: Rust core compiles from committed HEAD (no fallback) =="
rm -rf /tmp/bb11; git clone -q . /tmp/bb11; cd /tmp/bb11; mkdir -p bin
if (cd sandbox-core && cargo build --release >/tmp/bb11_rust.log 2>&1); then
  cp sandbox-core/target/release/burnbox-core bin/ && ok "core compiles from source"
else
  no "core FAILS to compile (see /tmp/bb11_rust.log)"
fi

echo "== [2] Packaged core contains M11 flags (proves no stale binary) =="
cd /tmp/bb11
./bin/burnbox-core --help 2>&1 | grep -q 'fs-jail' && ok "--fs-jail present" || no "--fs-jail MISSING (stale core?)"
./bin/burnbox-core --help 2>&1 | grep -q 'timeout' && ok "--timeout present" || no "--timeout MISSING (stale core?)"

echo "== [3] io_uring denied in default policy (runtime) =="
PY="$(command -v python3 || true)"
if [ -n "$PY" ]; then
  ./bin/burnbox-core --mode online --ram-mb 512 --target "$PY" -- -c \
   'import ctypes; libc=ctypes.CDLL("libc.so.6"); p=ctypes.create_string_buffer(120); libc.syscall(425,1,p)' >/dev/null 2>/dev/null; rc=$?
  [ $rc -eq 3 ] && ok "io_uring_setup -> 3" || no "io_uring rc=$rc (expected 3)"
else sk "no python3 for io_uring probe"; fi

echo "== [4] --timeout kills a hung target, scrubs, exits non-zero =="
t0=$SECONDS
./bin/burnbox-core --mode offline --ram-mb 512 --timeout 1 --target /bin/sleep -- 30 >/dev/null 2>/dev/null; rc=$?
el=$((SECONDS-t0))
if [ $rc -ne 0 ] && [ $el -le 3 ]; then ok "timeout killed sleep in ${el}s rc=$rc"; else no "timeout rc=$rc elapsed=${el}s"; fi
ls /dev/shm/ 2>/dev/null | grep -qi burnbox && no "LEAK after timeout" || ok "no /dev/shm leak after timeout"

echo "== [5] HONEST fs-jail negative (PASS only if jail engaged; SKIP if fallback) =="
echo "TOPSECRET_BB11" > /tmp/bb_hostsecret
out=$(./bin/burnbox-core --mode offline --ram-mb 512 --fs-jail --target /bin/cat -- /tmp/bb_hostsecret 2>/tmp/bb_jail_err.txt)
if grep -qiE 'falling back to overlay|host policy denied|pivot_root' /tmp/bb_jail_err.txt; then
  sk "fs-jail unavailable on this host (AppArmor) -> overlay; cannot prove jail live"
elif echo "$out" | grep -q 'TOPSECRET_BB11'; then
  no "JAIL LEAK: host /tmp readable while --fs-jail claimed active"
else
  ok "jail engaged: host /tmp hidden"
fi
rm -f /tmp/bb_hostsecret /tmp/bb_jail_err.txt

echo "== [6] Auto-discovery avoids snap shim =="
cd ~/projects/burnbox
./bin/burnbox open https://example.com --ram 512 >/tmp/bb11_url.log 2>&1; rc=$?
grep -qi 'snap-confine' /tmp/bb11_url.log && no "still launching snap shim" || ok "snap shim skipped (rc=$rc)"

echo "== [7] Acceptance is honest (no fake green) =="
./scripts/acceptance.sh >/tmp/bb11_acc.log 2>&1; rc=$?
[ $rc -eq 0 ] && ok "acceptance exit 0" || no "acceptance rc=$rc"
grep -qi 'ACCEPTANCE PASSED' /tmp/bb11_acc.log && ok "PASSED banner" || no "no banner"
grep -qiE 'Test 3.*PASS' /tmp/bb11_acc.log && grep -qi 'snap-confine' /tmp/bb11_acc.log && no "Test3 PASSed despite snap failure" || ok "Test3 not falsely green"

echo; echo "==== M11 GATE: $pass ok / $fail fail / $skip skip ===="
