#!/usr/bin/env bash
# Install solara into PREFIX (default ~/.local).
#   ./scripts/install.sh              -> ~/.local
#   PREFIX=/usr ./scripts/install.sh  -> /usr (run with sudo)
set -euo pipefail

PREFIX="${PREFIX:-$HOME/.local}"
BINDIR="${BINDIR:-$PREFIX/bin}"
LIBDIR="${LIBDIR:-$PREFIX/lib/solara}"
APPDIR="${APPDIR:-$PREFIX/share/applications}"
PORT="${PORT:-8787}"

cd "$(dirname "$0")/.."

echo "building release..."
cargo build --release
cargo build --release -p payload

echo "installing to $PREFIX"
install -d "$BINDIR" "$LIBDIR" "$APPDIR"
install -m 755 target/release/solara "$BINDIR/solara"
install -m 644 target/release/libpayload.so "$LIBDIR/libpayload.so"

sed -e "s|@BINDIR@|$BINDIR|g" \
    -e "s|@LIBDIR@|$LIBDIR|g" \
    -e "s|@PORT@|$PORT|g" \
    packaging/solara-ui.desktop > "$APPDIR/solara-ui.desktop"
chmod 644 "$APPDIR/solara-ui.desktop"

cat <<EOF

installed:
  $BINDIR/solara
  $LIBDIR/libpayload.so
  $APPDIR/solara-ui.desktop

next:
  1. make sure $BINDIR is on your PATH
  2. allow attach:   sudo sysctl kernel.yama.ptrace_scope=0
  3. launch:         solara ui

the payload is looked up next to the binary and in ../lib/solara,
so both 'solara' and 'sudo solara' find it.
EOF
