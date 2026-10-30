#!/usr/bin/env bash
# Remove an installation created by scripts/install.sh.
set -euo pipefail

PREFIX="${PREFIX:-$HOME/.local}"
BINDIR="${BINDIR:-$PREFIX/bin}"
LIBDIR="${LIBDIR:-$PREFIX/lib/solara}"
APPDIR="${APPDIR:-$PREFIX/share/applications}"

removed=0
for f in "$BINDIR/solara" "$APPDIR/solara-ui.desktop"; do
    if [ -e "$f" ]; then
        rm -f "$f"
        echo "removed $f"
        removed=$((removed+1))
    fi
done

if [ -d "$LIBDIR" ]; then
    rm -rf "$LIBDIR"
    echo "removed $LIBDIR"
    removed=$((removed+1))
fi

if [ "$removed" -eq 0 ]; then
    echo "nothing to remove under $PREFIX"
else
    echo
    echo "note: this does not touch kernel.yama.ptrace_scope."
    echo "      restore it with: sudo sysctl kernel.yama.ptrace_scope=1"
fi
