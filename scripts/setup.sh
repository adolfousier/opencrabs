#!/usr/bin/env bash
set -euo pipefail

# OpenCrabs — build-from-source setup script
# Detects platform, installs system dependencies, and ensures Rust nightly is ready.

BOLD='\033[1m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
RED='\033[0;31m'
RESET='\033[0m'

info()  { echo -e "${BOLD}[info]${RESET}  $*"; }
ok()    { echo -e "${GREEN}[ok]${RESET}    $*"; }
warn()  { echo -e "${YELLOW}[warn]${RESET}  $*"; }
error() { echo -e "${RED}[error]${RESET} $*"; exit 1; }

# ---------------------------------------------------------------------------
# 1. Detect OS and distro
# ---------------------------------------------------------------------------

OS="$(uname -s)"
ARCH="$(uname -m)"
DISTRO=""

detect_distro() {
    if [ -f /etc/os-release ]; then
        # shellcheck disable=SC1091
        . /etc/os-release
        DISTRO="${ID}"
    fi
}

info "Detected: ${OS} ${ARCH}"

# ---------------------------------------------------------------------------
# 2. Install system dependencies
# ---------------------------------------------------------------------------

install_macos() {
    info "Platform: macOS"

    # Xcode Command Line Tools
    if ! xcode-select -p &>/dev/null; then
        info "Installing Xcode Command Line Tools..."
        xcode-select --install
        echo "  -> After the install dialog completes, re-run this script."
        exit 0
    else
        ok "Xcode CLI Tools already installed"
    fi

    # Homebrew
    if ! command -v brew &>/dev/null; then
        error "Homebrew not found. Install it first: https://brew.sh"
    fi

    info "Installing cmake and pkg-config via Homebrew..."
    brew install cmake pkg-config
    ok "macOS dependencies installed"
}

install_debian() {
    info "Platform: Debian/Ubuntu"
    sudo apt-get update -qq
    sudo apt-get install -y build-essential pkg-config libssl-dev cmake
    ok "Debian/Ubuntu dependencies installed"
}

install_fedora() {
    info "Platform: Fedora/RHEL"
    sudo dnf install -y gcc gcc-c++ make pkg-config openssl-devel cmake
    ok "Fedora/RHEL dependencies installed"
}

install_arch() {
    info "Platform: Arch Linux"
    sudo pacman -S --needed --noconfirm base-devel pkg-config openssl cmake
    ok "Arch dependencies installed"
}

case "${OS}" in
    Darwin)
        install_macos
        ;;
    Linux)
        detect_distro
        case "${DISTRO}" in
            ubuntu|debian|pop|linuxmint|elementary)
                install_debian
                ;;
            fedora|rhel|centos|rocky|alma)
                install_fedora
                ;;
            arch|manjaro|endeavouros)
                install_arch
                ;;
            *)
                if [ -n "${DISTRO}" ]; then
                    warn "Unknown distro '${DISTRO}'. Trying apt-get..."
                    install_debian
                else
                    error "Cannot detect Linux distro. Install manually: build-essential pkg-config libssl-dev cmake"
                fi
                ;;
        esac
        ;;
    *)
        error "Unsupported OS: ${OS}. On Windows, use WSL2 with Ubuntu and re-run this script."
        ;;
esac

# ---------------------------------------------------------------------------
# 3. Install / verify Rust nightly
# ---------------------------------------------------------------------------

if ! command -v rustup &>/dev/null; then
    info "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # shellcheck disable=SC1091
    source "${HOME}/.cargo/env"
    ok "Rust installed"
else
    ok "rustup already installed"
fi

if ! rustup toolchain list | grep -q nightly; then
    info "Installing Rust nightly toolchain..."
    rustup toolchain install nightly
    ok "Nightly toolchain installed"
else
    ok "Rust nightly already installed"
fi

# ---------------------------------------------------------------------------
# 4. Verify everything
# ---------------------------------------------------------------------------

echo ""
info "Verification:"

check() {
    if command -v "$1" &>/dev/null; then
        ok "$1 — $("$1" --version 2>/dev/null || echo 'ok')"
    else
        error "$1 not found — something went wrong"
    fi
}

check cmake
check pkg-config
check cargo
check rustc

echo ""
echo -e "${GREEN}${BOLD}Setup complete!${RESET}"
echo ""
echo "Next steps:"
echo "  git clone https://github.com/adolfousier/opencrabs.git"
echo "  cd opencrabs"
echo "  cargo build --release"
echo "  ./target/release/opencrabs"
echo ""
