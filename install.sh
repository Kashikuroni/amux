#!/bin/sh
# amux installer — downloads the latest prebuilt macOS binary from GitHub
# Releases, clears the Gatekeeper quarantine, and installs it onto your PATH.
#
#   curl -fsSL https://raw.githubusercontent.com/Kashikuroni/amux/main/install.sh | sh
#
# Overrides (env vars):
#   AMUX_VERSION=v0.1.0     install a specific tag instead of the latest
#   AMUX_BIN_DIR=/path      install dir (default: /usr/local/bin if writable, else ~/.local/bin)
set -eu

REPO="Kashikuroni/amux"
BIN="amux"

err() { printf 'error: %s\n' "$1" >&2; exit 1; }

# --- platform ----------------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
[ "$os" = "Darwin" ] || err "amux currently supports macOS only (got $os)."
case "$arch" in
  arm64 | aarch64) target="aarch64-apple-darwin" ;;
  x86_64) target="x86_64-apple-darwin" ;;
  *) err "unsupported architecture: $arch" ;;
esac

command -v curl >/dev/null 2>&1 || err "curl is required."
command -v tar  >/dev/null 2>&1 || err "tar is required."

# --- version (latest unless AMUX_VERSION is set) -----------------------------
version="${AMUX_VERSION:-}"
if [ -z "$version" ]; then
  version="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
fi
[ -n "$version" ] || err "could not determine the latest version; set AMUX_VERSION (e.g. v0.1.0)."

asset="${BIN}-${version}-${target}.tar.gz"
url="https://github.com/$REPO/releases/download/$version/$asset"

# --- install dir -------------------------------------------------------------
bindir="${AMUX_BIN_DIR:-}"
if [ -z "$bindir" ]; then
  if [ -w /usr/local/bin ]; then bindir="/usr/local/bin"; else bindir="$HOME/.local/bin"; fi
fi
mkdir -p "$bindir" || err "cannot create install dir: $bindir"

# --- download + install ------------------------------------------------------
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

printf 'Downloading %s %s (%s)…\n' "$BIN" "$version" "$target"
curl -fSL --progress-bar "$url" -o "$tmp/$asset" || err "download failed: $url"
tar -xzf "$tmp/$asset" -C "$tmp" || err "could not extract $asset"
[ -f "$tmp/$BIN" ] || err "archive did not contain a '$BIN' binary."

# Unsigned binary → clear the quarantine flag so Gatekeeper doesn't block it.
xattr -d com.apple.quarantine "$tmp/$BIN" 2>/dev/null || true
chmod +x "$tmp/$BIN"
mv "$tmp/$BIN" "$bindir/$BIN" || err "could not install to $bindir (try sudo, or set AMUX_BIN_DIR)."

printf '\nInstalled %s %s → %s\n' "$BIN" "$version" "$bindir/$BIN"
case ":$PATH:" in
  *":$bindir:"*) printf 'Run: %s\n' "$BIN" ;;
  *) printf 'Add %s to your PATH:\n  export PATH="%s:$PATH"\n' "$bindir" "$bindir" ;;
esac
