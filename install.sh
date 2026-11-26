#!/usr/bin/env sh
set -eu

REPO="${ECLIVEN_REPO:-ikiresearch/ecliven-code}"
INSTALL_DIR="${ECLIVEN_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${ECLIVEN_VERSION:-}"

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)
    case "$ARCH" in
      x86_64|amd64) TARGET="x86_64-unknown-linux-musl" ;;
      *) echo "Unsupported Linux architecture: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  Darwin)
    case "$ARCH" in
      arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
      x86_64) TARGET="x86_64-apple-darwin" ;;
      *) echo "Unsupported macOS architecture: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "Unsupported OS: $OS. On Windows use install.ps1." >&2
    exit 1
    ;;
esac

ASSET="ecliven-${TARGET}.tar.gz"
if [ -n "$VERSION" ]; then
  case "$VERSION" in
    v*) TAG="$VERSION" ;;
    *) TAG="v$VERSION" ;;
  esac
  BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"
else
  BASE_URL="https://github.com/${REPO}/releases/latest/download"
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Downloading Ecliven Code from ${BASE_URL}/${ASSET} ..."
if ! curl -fsSL "${BASE_URL}/${ASSET}" -o "${TMP_DIR}/${ASSET}"; then
  if [ "$TARGET" = "x86_64-unknown-linux-musl" ]; then
    # Releases before the static Linux build only ship the glibc binary.
    TARGET="x86_64-unknown-linux-gnu"
    ASSET="ecliven-${TARGET}.tar.gz"
    echo "Static build not found in this release, downloading ${ASSET} ..."
    curl -fsSL "${BASE_URL}/${ASSET}" -o "${TMP_DIR}/${ASSET}"
  else
    exit 1
  fi
fi

mkdir -p "$INSTALL_DIR"
tar -xzf "${TMP_DIR}/${ASSET}" -C "$TMP_DIR"
install -m 755 "${TMP_DIR}/ecliven" "${INSTALL_DIR}/ecliven"

echo "Installed Ecliven Code to ${INSTALL_DIR}/ecliven"
case ":$PATH:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    echo "Add this to your shell profile (~/.bashrc or ~/.zshrc):"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    ;;
esac
echo "Run 'ecliven' to start."
