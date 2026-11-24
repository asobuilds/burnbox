#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
pass=0; fail=0
ok(){ echo "  ✅ $1"; pass=$((pass+1)); }
no(){ echo "  ❌ $1"; fail=$((fail+1)); }

echo "== [1] Version is 1.0.0 =="
out="$(./bin/burnbox --version 2>&1)"
echo "$out" | grep -q '1.0.0' && ok "version 1.0.0" || no "version mismatch: $out"

echo "== [2] Doctor checks slirp4netns readiness =="
./bin/burnbox doctor >/tmp/bb9_doc.log 2>&1
grep -qi 'slirp' /tmp/bb9_doc.log && ok "doctor reports slirp status" || no "doctor missing slirp check"

echo "== [3] Symlink escape prevention (Security) =="
ln -sf /etc/hostname /tmp/bb9_evil_link
./bin/burnbox-core --mode offline --ram-mb 512 --copy-in /tmp/bb9_evil_link --target /bin/cat -- bb9_evil_link >/tmp/bb9_cat_out.txt 2>/tmp/bb9_cat_err.txt || true
# If it copied the symlink, cat will output the symlink text or fail. If it escaped, it outputs the actual hostname.
# We check that the output does NOT match the real hostname, or that it explicitly rejected the symlink.
real_host=$(cat /etc/hostname | tr -d '\n')
cat_out=$(cat /tmp/bb9_cat_out.txt | tr -d '\n' || echo "")
if [ "$cat_out" != "$real_host" ] || grep -qi 'symlink\|refused\|rejected' /tmp/bb9_cat_err.txt; then
  ok "symlink escape prevented"
else
  no "SECURITY FAIL: symlink resolved to host file!"
fi
rm -f /tmp/bb9_evil_link /tmp/bb9_cat_out.txt /tmp/bb9_cat_err.txt

echo "== [4] Acceptance pipeline still green =="
./scripts/acceptance.sh >/tmp/bb9_acc.log 2>&1; rc=$?
[ $rc -eq 0 ] && ok "acceptance.sh exit 0" || no "acceptance.sh rc=$rc"

echo; echo "==== VERDICT: $pass ok / $fail fail ===="
