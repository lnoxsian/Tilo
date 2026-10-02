#!/usr/bin/env bash
# ==============================================================================
# tilo - Dependency & Rust Toolchain Installer
# ==============================================================================
# Installs all system build dependencies, C libraries (Cairo, Pango, Fontconfig),
# runtime debug tools (Xephyr, xterm, x11-apps), and the Rust toolchain via rustup.
# Supports: Debian / Ubuntu / Linux Mint, Arch Linux, Fedora, openSUSE, and Alpine.
# ==============================================================================

set -euo pipefail

# ANSI Color Codes
C_RESET=$'\033[0m'
C_BOLD=$'\033[1m'
C_RED=$'\033[31m'
C_GREEN=$'\033[32m'
C_YELLOW=$'\033[33m'
C_CYAN=$'\033[36m'
C_GRAY=$'\033[90m'

echo -e "${C_BOLD}${C_CYAN}=====================================================================${C_RESET}"
echo -e "${C_BOLD} tilo - Installing Build & Debug Dependencies + Rust Toolchain${C_RESET}"
echo -e "${C_BOLD}${C_CYAN}=====================================================================${C_RESET}"
echo ""

# Helper function to run commands with sudo if not root
run_sudo() {
    if [[ $EUID -eq 0 ]]; then
        "$@"
    elif command -v sudo >/dev/null 2>&1; then
        sudo "$@"
    else
        echo -e "${C_RED}Error: Root privileges required. Please install sudo or run as root.${C_RESET}" >&2
        exit 1
    fi
}

# 1. Detect Linux Distribution
echo -e "${C_CYAN}--> [1/3] Detecting Linux distribution...${C_RESET}"

DISTRO=""
if [[ -f /etc/os-release ]]; then
    . /etc/os-release
    DISTRO="${ID:-}"
    DISTRO_LIKE="${ID_LIKE:-}"
fi

case "${DISTRO,,}" in
    ubuntu|debian|linuxmint|pop|elementary|zorin|kali)
        DISTRO_FAMILY="debian"
        ;;
    arch|manjaro|endeavouros|garuda|artix)
        DISTRO_FAMILY="arch"
        ;;
    fedora|rhel|centos|rocky|alma)
        DISTRO_FAMILY="fedora"
        ;;
    opensuse*|suse)
        DISTRO_FAMILY="suse"
        ;;
    alpine)
        DISTRO_FAMILY="alpine"
        ;;
    *)
        if [[ "${DISTRO_LIKE,,}" =~ debian|ubuntu ]]; then
            DISTRO_FAMILY="debian"
        elif [[ "${DISTRO_LIKE,,}" =~ arch ]]; then
            DISTRO_FAMILY="arch"
        elif [[ "${DISTRO_LIKE,,}" =~ fedora|rhel ]]; then
            DISTRO_FAMILY="fedora"
        else
            DISTRO_FAMILY="unknown"
        fi
        ;;
esac

echo -e "    Detected: ${C_BOLD}${NAME:-Unknown}${C_RESET} (Family: ${DISTRO_FAMILY})"

# 2. Install System Build & Runtime Debug Dependencies
echo ""
echo -e "${C_CYAN}--> [2/3] Installing system packages (C development headers & Xephyr)...${C_RESET}"

case "${DISTRO_FAMILY}" in
    debian)
        echo -e "${C_GRAY}    Running: apt-get update && apt-get install...${C_RESET}"
        run_sudo apt-get update -y
        run_sudo apt-get install -y --no-install-recommends \
            build-essential \
            curl \
            git \
            make \
            pkg-config \
            libcairo2-dev \
            libpango1.0-dev \
            libfontconfig1-dev \
            libfreetype6-dev \
            xserver-xephyr \
            x11-apps \
            xterm
        ;;
    arch)
        echo -e "${C_GRAY}    Running: pacman -Syu --needed...${C_RESET}"
        run_sudo pacman -Syu --needed --noconfirm \
            base-devel \
            curl \
            git \
            make \
            pkgconf \
            cairo \
            pango \
            fontconfig \
            freetype2 \
            xorg-server-xephyr \
            xorg-xcalc \
            xorg-xclock \
            xterm
        ;;
    fedora)
        echo -e "${C_GRAY}    Running: dnf install...${C_RESET}"
        run_sudo dnf install -y \
            gcc \
            make \
            curl \
            git \
            pkgconf-pkg-config \
            cairo-devel \
            pango-devel \
            fontconfig-devel \
            freetype-devel \
            xorg-x11-server-Xephyr \
            xorg-x11-apps \
            xterm
        ;;
    suse)
        echo -e "${C_GRAY}    Running: zypper install...${C_RESET}"
        run_sudo zypper refresh
        run_sudo zypper install -y \
            gcc \
            make \
            curl \
            git \
            pkg-config \
            cairo-devel \
            pango-devel \
            fontconfig-devel \
            freetype2-devel \
            xorg-x11-server-extra \
            xterm
        ;;
    alpine)
        echo -e "${C_GRAY}    Running: apk add...${C_RESET}"
        run_sudo apk update
        run_sudo apk add \
            build-base \
            curl \
            git \
            make \
            pkgconf \
            cairo-dev \
            pango-dev \
            fontconfig-dev \
            freetype-dev \
            xserver-xephyr \
            xterm
        ;;
    *)
        echo -e "${C_YELLOW}Warning: Unrecognized distribution '${DISTRO}'. Attempting generic package installation...${C_RESET}" >&2
        echo "Please ensure you have: gcc, make, pkg-config, cairo-dev, pango-dev, and xserver-xephyr installed." >&2
        ;;
esac

echo -e "${C_GREEN}    [✓] System packages installed successfully.${C_RESET}"

# 3. Install or Verify Rust Toolchain
echo ""
echo -e "${C_CYAN}--> [3/3] Checking Rust & Cargo toolchain...${C_RESET}"

# Check if cargo is currently available
if command -v cargo >/dev/null 2>&1 && command -v rustc >/dev/null 2>&1; then
    CURRENT_RUSTC="$(rustc --version)"
    echo -e "    Rust is already installed: ${C_BOLD}${CURRENT_RUSTC}${C_RESET}"
else
    # Check if rustup environment exists in home directory
    if [[ -f "$HOME/.cargo/env" ]]; then
        # shellcheck source=/dev/null
        source "$HOME/.cargo/env"
    fi

    if command -v cargo >/dev/null 2>&1; then
        CURRENT_RUSTC="$(rustc --version)"
        echo -e "    Rust is already installed in ~/.cargo/bin: ${C_BOLD}${CURRENT_RUSTC}${C_RESET}"
    else
        echo -e "${C_YELLOW}    Rust not found. Installing Rust toolchain via rustup.rs...${C_RESET}"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
        # shellcheck source=/dev/null
        source "$HOME/.cargo/env"
        echo -e "${C_GREEN}    [✓] Rust installed: $(rustc --version)${C_RESET}"
    fi
fi

# Ensure rustc version is recent enough for 2024 edition (1.85+)
RUSTC_VER_STR="$(rustc --version | awk '{print $2}')"
RUSTC_MAJOR="$(echo "$RUSTC_VER_STR" | cut -d. -f1)"
RUSTC_MINOR="$(echo "$RUSTC_VER_STR" | cut -d. -f2)"

if [[ "$RUSTC_MAJOR" -eq 1 && "$RUSTC_MINOR" -lt 85 ]]; then
    echo -e "${C_YELLOW}    Rust version ${RUSTC_VER_STR} is older than 1.85. Updating via rustup...${C_RESET}"
    if command -v rustup >/dev/null 2>&1; then
        rustup update stable
        echo -e "${C_GREEN}    [✓] Rust updated to: $(rustc --version)${C_RESET}"
    else
        echo -e "${C_RED}Warning: Rust 1.85+ is required for edition 2024. Please update rustc.${C_RESET}" >&2
    fi
fi

echo ""
echo -e "${C_BOLD}${C_GREEN}=====================================================================${C_RESET}"
echo -e "${C_BOLD}${C_GREEN} All dependencies and Rust toolchain are ready!${C_RESET}"
echo -e "${C_BOLD}${C_GREEN}=====================================================================${C_RESET}"
echo ""
echo -e "You can now build, test, and run tilo locally:"
echo -e "  1. Build debug binary:               ${C_CYAN}make debug${C_RESET} (or ${C_CYAN}cargo build${C_RESET})"
echo -e "  2. Run Xephyr interactive session:   ${C_CYAN}make test-xephyr${C_RESET}"
echo -e "  3. Run Xephyr layout demo:           ${C_CYAN}make test-layout${C_RESET}"
echo -e "  4. Build optimized release binary:   ${C_CYAN}make release${C_RESET}"
echo -e "  5. Build via Docker:                 ${C_CYAN}make docker-build${C_RESET}"
echo ""
