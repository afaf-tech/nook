#!/usr/bin/env bash
# Install Nook from this checkout: build the two release binaries and put them on PATH.
#
#   ./install.sh                     # into ~/.local/bin
#   PREFIX=/usr/local ./install.sh   # somewhere else
set -euo pipefail

cd "$(dirname "$0")"

PREFIX="${PREFIX:-$HOME/.local/bin}"

if ! command -v cargo >/dev/null 2>&1; then
  echo "install.sh: cargo not found — install Rust from https://rustup.rs and try again." >&2
  exit 1
fi

echo "== build =="
cargo build --release -p nook -p nook-cli

echo "== install =="
mkdir -p "$PREFIX"
install -m 755 target/release/nook target/release/nook-cli "$PREFIX/"
echo "installed: $PREFIX/nook"
echo "           $PREFIX/nook-cli"

case ":$PATH:" in
  *":$PREFIX:"*) ;;
  *) echo "note: $PREFIX is not on PATH — add it in your shell profile to run 'nook' by name." ;;
esac

echo "run: nook --demo      the desktop app, on a sample database"
echo "     nook-cli help    every command from the shell"
