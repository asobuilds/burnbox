#!/usr/bin/env bash
cd ~/projects/burnbox || { echo "repo missing"; exit 1; }
PY="$(command -v python3 || true)"; pass=0; fail=0
ok(){ echo "  ✅ $1"; pass=$((pass+1)); }
no(){ echo "  ❌ $1"; fail=$((fail+1)); }

echo "== [1] --version =="
out="$(./bin/burnbox --version 2>&1)"; rc=$?; echo "  got: $out (rc=$rc)"
if [ $rc -eq 0 ] && echo "$out" | grep -q 'v0.5.0'; then ok "version v0.5.0"; else no "version"; fi

echo "== [2] offline lifecycle /bin/true =="
./bin/burnbox open /bin/true --offline --ram 512 >/tmp/bb2.json 2>/tmp/bb2.err; rc=$?
echo "  rc=$rc"; cat /tmp/bb2.json
if [ $rc -eq 0 ] && [ "$(grep -c '"status"' /tmp/bb2.json)" -eq 2 ]; then ok "exit0 + 2 json lines"; else no "lifecycle"; fi
if ls /dev/shm 2>/dev/null | grep -qi burnbox; then no "LEAK /dev/shm"; else ok "no /dev/shm leftovers"; fi

echo "== [3] copy-in staging scrub =="
echo "secret" > /tmp/invoice_test.pdf
./bin/burnbox open /tmp/invoice_test.pdf --offline --ram 512 >/tmp/bb3.json 2>/tmp/bb3.err; rc=$?
echo "  rc=$rc"
if [ $rc -eq 0 ]; then ok "copy-in exit0"; else no "copy-in rc=$rc"; fi
if ls /dev/shm 2>/dev/null | grep -qi burnbox; then no "STAGING LEAK"; else ok "staged file scrubbed"; fi

echo "== [4] seccomp NEGATIVE (ptrace must be killed -> 3) =="
if [ -n "$PY" ]; then
  ./bin/burnbox-core --mode online --ram-mb 512 --target "$PY" -- -c 'import ctypes;ctypes.CDLL("libc.so.6").ptrace(0,0,0,0)' >/tmp/bb4.json 2>/dev/null; rc=$?
  echo "  rc=$rc (expect 3)"; if [ $rc -eq 3 ]; then ok "ptrace killed -> 3"; else no "ptrace rc=$rc (filter NOT live?)"; fi
else echo "  ⚠️ python3 missing; skipped"; fi

echo "== [5] seccomp COMPAT (normal read must NOT be killed) =="
if [ -n "$PY" ]; then
  ./bin/burnbox-core --mode online --ram-mb 512 --target "$PY" -- -c 'print(open("/etc/hostname").read().strip())' >/tmp/bb5.json 2>/dev/null; rc=$?
  echo "  rc=$rc (expect 0, NOT 3)"; if [ $rc -eq 0 ]; then ok "compat ok"; else no "compat rc=$rc -> FILTER OVER-BROAD, STOP"; fi
else echo "  ⚠️ python3 missing; skipped"; fi

echo "== [6] packaging + rootless postinst =="
./scripts/package.sh >/tmp/bb6.log 2>&1; rc=$?; ls -1 release/ 2>/dev/null
if [ $rc -eq 0 ] && ls release/*.deb >/dev/null 2>&1 && ls release/*.tar.gz >/dev/null 2>&1; then ok "deb + tar.gz"; else no "package.sh rc=$rc"; fi
deb="$(ls release/*.deb 2>/dev/null | head -n1)"
if [ -n "$deb" ]; then
  rm -rf /tmp/bbdeb; mkdir -p /tmp/bbdeb; dpkg-deb -e "$deb" /tmp/bbdeb 2>/dev/null
  if grep -qi 'sudo' /tmp/bbdeb/postinst 2>/dev/null; then no "postinst uses sudo"; else ok "no sudo in postinst"; fi
  if grep -Eq '(cp|install|mkdir|touch|>>?)[[:space:]].*(/usr/|/etc/)' /tmp/bbdeb/postinst 2>/dev/null; then no "postinst WRITES /usr|/etc"; else ok "postinst user-space only (calling /usr/bin/xdg-mime is fine)"; fi
fi

echo "== [7] desktop-file test + MIME/Exec =="
go test ./internal/cli/ -run Desktop -v 2>&1 | tail -n 4
grep -E 'MimeType=|Exec=burnbox open' packaging/burnbox.desktop

echo; echo "==== VERDICT: $pass ok / $fail fail ===="
