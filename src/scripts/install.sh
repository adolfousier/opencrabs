#!/usr/bin/env bash
set -euo pipefail

# OpenCrabs — one-line install
# curl -fsSL https://raw.githubusercontent.com/adolfousier/opencrabs/main/src/scripts/install.sh | bash

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}🦀${NC} $*"; }
warn()  { echo -e "${YELLOW}⚠️${NC}  $*"; }
error() { echo -e "${RED}❌${NC} $*" >&2; exit 1; }

# Detect OS and arch
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$ARCH" in
  x86_64)  ARCH="amd64" ;;
  aarch64) ARCH="arm64" ;;
  arm64)   ARCH="arm64" ;;
  *)       error "Unsupported architecture: $ARCH" ;;
esac

case "$OS" in
  linux)  EXT="tar.gz" ;;
  darwin) EXT="tar.gz" ;;
  *)      error "Unsupported OS: $OS (linux and darwin only)" ;;
esac

INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

# Check if install dir is writable
if [ ! -w "$INSTALL_DIR" ] 2>/dev/null; then
  SUDO="sudo"
  warn "Need sudo to install to $INSTALL_DIR"
else
  SUDO=""
fi

info "Detecting latest release..."
TAG=$(curl -fsSL https://api.github.com/repos/adolfousier/opencrabs/releases/latest \
  | grep -o '"tag_name": *"[^"]*"' \
  | head -1 \
  | cut -d'"' -f4)

if [ -z "$TAG" ]; then
  error "Could not determine latest release tag"
fi

FILENAME="opencrabs-${TAG}-${OS}-${ARCH}.tar.gz"
DOWNLOAD_URL="https://github.com/adolfousier/opencrabs/releases/download/${TAG}/${FILENAME}"

info "Downloading ${TAG} for ${OS}-${ARCH}..."
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

if ! curl -fsSL "$DOWNLOAD_URL" -o "${TMPDIR}/${FILENAME}"; then
  error "Failed to download ${FILENAME}\n   URL: ${DOWNLOAD_URL}\n   Check https://github.com/adolfousier/opencrabs/releases for available releases"
fi

info "Extracting..."
tar xzf "${TMPDIR}/${FILENAME}" -C "$TMPDIR"

info "Installing to ${INSTALL_DIR}..."
$SUDO install -m 755 "${TMPDIR}/opencrabs" "${INSTALL_DIR}/opencrabs"

info "OpenCrabs ${TAG} installed to ${INSTALL_DIR}/opencrabs"
info "Run 'opencrabs' to get started!"
