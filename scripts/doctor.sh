#!/usr/bin/env bash
# Preflight check for solara. Run before attaching to anything.
set -uo pipefail

ok=0
warn=0
err=0

pass() { printf '  \033[32mok\033[0m    %s\n' "$1"; ok=$((ok+1)); }
note() { printf '  \033[33mwarn\033[0m  %s\n' "$1"; warn=$((warn+1)); }
fail() { printf '  \033[31merr\033[0m   %s\n' "$1"; err=$((err+1)); }

echo
echo "solara doctor"
echo "-------------"

# 1. binary + payload
bin="$(command -v solara 2>/dev/null || true)"
if [ -z "$bin" ] && [ -x ./target/release/solara ]; then
    bin="./target/release/solara"
fi
if [ -n "$bin" ]; then
    pass "binary: $bin"
else
    fail "binary not found — run 'make' or 'make install'"
fi

payload=""
for p in \
    "$(dirname "${bin:-.}")/libpayload.so" \
    "$(dirname "${bin:-.}")/../lib/solara/libpayload.so" \
    ./target/release/libpayload.so \
    ./target/debug/libpayload.so
do
    [ -f "$p" ] && { payload="$p"; break; }
done
if [ -n "$payload" ]; then
    pass "payload: $payload"
else
    fail "libpayload.so not found — run 'make payload'"
fi

# 2. ptrace gate
scope="$(cat /proc/sys/kernel/yama/ptrace_scope 2>/dev/null || echo missing)"
if [ "$scope" = "0" ]; then
    pass "ptrace_scope = 0 (free attach)"
elif [ "$scope" = "missing" ]; then
    note "yama ptrace_scope not present; kernel may allow attach"
else
    fail "ptrace_scope = $scope — only parent may attach"
    echo "         fix: sudo sysctl kernel.yama.ptrace_scope=0"
    echo "         or : sudo ./solara ui"
fi

# 3. capability check
if [ "$(id -u)" -eq 0 ]; then
    pass "running as root (ptrace unrestricted)"
else
    note "running as user $(id -un); sandboxed targets need root or scope=0"
fi

# 4. runtime helpers
if command -v xdg-open >/dev/null 2>&1; then
    pass "xdg-open present (ui auto-open)"
else
    note "xdg-open missing; open the printed URL manually"
fi

if command -v flatpak >/dev/null 2>&1; then
    pass "flatpak present"
    if flatpak list 2>/dev/null | grep -qi sober; then
        pass "sober flatpak installed"
        if flatpak ps 2>/dev/null | grep -qi sober; then
            pass "sober is running"
        else
            note "sober installed but not running"
        fi
    else
        note "sober flatpak not installed"
    fi
else
    note "flatpak not installed (only needed for Sober)"
fi

# 5. live targets
if [ -n "$bin" ]; then
    out="$("$bin" find 2>/dev/null)"
    if echo "$out" | grep -q 'no roblox'; then
        note "no target process running"
    else
        pass "targets visible:"
        echo "$out" | sed 's/^/         /'
    fi
fi

# 6. cargo toolchain
if command -v cargo >/dev/null 2>&1; then
    pass "cargo $(cargo --version 2>/dev/null | awk '{print $2}')"
else
    note "cargo missing (only needed to build from source)"
fi

echo "-------------"
printf '  %d ok, %d warn, %d err\n\n' "$ok" "$warn" "$err"

if [ "$err" -gt 0 ]; then
    echo "  resolve the errors above before attaching."
    exit 1
fi
echo "  ready. next: solara ui"
exit 0
