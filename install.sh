#!/usr/bin/env sh
# Installs the latest `ciac` release binary into ~/.local/bin.
#
# Usage: curl -fsSL https://raw.githubusercontent.com/jrcalgo/ciac/main/install.sh | sh
#
# Downloads the release asset matching this machine's OS/arch from
# the latest GitHub release of jrcalgo/ciac (see
# .github/workflows/release.yml for how assets are named and built),
# verifies it's executable, and installs it as `ciac` in
# $HOME/.local/bin (create the directory and add it to $PATH if it
# isn't already).

set -eu

REPO="jrcalgo/ciac"
INSTALL_DIR="${CIAC_INSTALL_DIR:-$HOME/.local/bin}"

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
    Linux) platform="linux" ;;
    Darwin) platform="macos" ;;
    *)
        echo "error: unsupported OS '$os' (this script installs Linux/macOS binaries;" >&2
        echo "Windows users should download the .exe asset from the latest release instead:" >&2
        echo "  https://github.com/$REPO/releases/latest" >&2
        exit 1
        ;;
esac

case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *)
        echo "error: unsupported architecture '$arch'" >&2
        exit 1
        ;;
esac

asset="ciac-${platform}-${arch}"
url="https://github.com/$REPO/releases/latest/download/$asset"

echo "downloading $url"
mkdir -p "$INSTALL_DIR"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$tmp"
elif command -v wget >/dev/null 2>&1; then
    wget -q "$url" -O "$tmp"
else
    echo "error: need curl or wget to download the release" >&2
    exit 1
fi

chmod +x "$tmp"
mv "$tmp" "$INSTALL_DIR/ciac"
trap - EXIT

echo "installed ciac to $INSTALL_DIR/ciac"
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        echo "note: $INSTALL_DIR is not on \$PATH — add it, e.g.:"
        echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
        ;;
esac
"$INSTALL_DIR/ciac" --version
