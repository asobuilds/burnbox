#!/usr/bin/env bash
set -u
cd "$(dirname "$0")"
pass=0; fail=0; skip=0
ok(){ echo "  ✅ $1"; pass=$((pass+1)); }
no(){ echo "  ❌ $1"; fail=$((fail+1)); }
sk(){ echo "  ⏭️  $1"; skip=$((skip+1)); }

echo "== [A] version coherence (source=binary=artifact) =="
SRC=$(grep -oP 'Version\s*=\s*"\K[^"]+' internal/version/version.go)
BIN=$(./bin/burnbox --version | grep -oP 'v\K[0-9.]+')
ART=$(ls -1 release/ | grep -oP 'burnbox_\K[0-9.]+(?=_amd64\.deb)' | head -1)
echo "  source=$SRC binary=$BIN artifact=$ART"
[ "$SRC" = "$BIN" ] && [ "$SRC" = "$ART" ] && ok "coherent $SRC" || no "DRIFT"

echo "== [B] user flow: open a local file OFFLINE =="
echo "hello burnbox" > /tmp/v101.txt
./bin/burnbox open /tmp/v101.txt --offline --ram 512 --copy-in /tmp/v101.txt >/tmp/v101_b.log 2>&1; rc=$?
[ $rc -eq 0 ] && ok "offline open rc=0" || no "offline rc=$rc"
grep -q '"wiped"' /tmp/v101_b.log && ok "offline emitted wiped" || no "no wiped line"
ls /dev/shm/ 2>/dev/null | grep -qi burnbox && no "offline LEAK" || ok "offline no leak"

echo "== [C] user flow: open a local file ONLINE =="
./bin/burnbox open /tmp/v101.txt --ram 512 --copy-in /tmp/v101.txt >/tmp/v101_c.log 2>&1; rc=$?
[ $rc -eq 0 ] && ok "online open rc=0" || no "online rc=$rc"

echo "== [D] user flow: open a URL (must not fake-launch snap shim) =="
./bin/burnbox open https://example.com --ram 512 >/tmp/v101_d.log 2>&1; rc=$?
grep -qi 'snap-confine' /tmp/v101_d.log && no "launched snap shim" || ok "snap shim skipped (rc=$rc)"
if command -v slirp4netns >/dev/null 2>&1; then ok "slirp present (live URL possible)"; else sk "slirp absent -> loopback-only by design"; fi

echo "== [E] timeout path scrubs =="
./bin/burnbox-core --mode offline --ram-mb 512 --timeout 1 --target /bin/sleep -- 30 >/dev/null 2>&1; rc=$?
[ $rc -ne 0 ] && ok "timeout rc=$rc" || no "timeout rc=0 (should be non-zero)"
ls /dev/shm/ 2>/dev/null | grep -qi burnbox && no "timeout LEAK" || ok "timeout no leak"

echo "== [F] doctor runs 7 checks =="
./bin/burnbox doctor >/tmp/v101_f.log 2>&1; rc=$?
n=$(grep -cE '\[(PASS|FAIL|WARN)\]' /tmp/v101_f.log)
[ $n -ge 7 ] && ok "doctor $n checks" || no "doctor only $n checks"

echo "== [G] artifacts correct =="
tar tzf release/burnbox_${SRC}_amd64.tar.gz 2>/dev/null | grep -q 'bin/burnbox-core' && ok "tarball ships core" || no "tarball missing core"
dpkg-deb -e release/burnbox_${SRC}_amd64.deb /tmp/v101deb 2>/dev/null
grep -q '\$HOME' /tmp/v101deb/postinst && no "deb postinst uses \$HOME" || ok "deb postinst root-safe"
grep -q 'x-scheme-handler/http' packaging/burnbox.desktop && ok "desktop http handler" || no "desktop missing http"

echo; echo "==== v1.0.1 FUNCTIONAL: $pass ok / $fail fail / $skip skip ===="
