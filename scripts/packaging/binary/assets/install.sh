#!/usr/bin/env sh
# ==============================================================================
# install.sh - Standalone Installer for Tilo Window Manager
# ==============================================================================
# Installs Tilo binary, XSession desktop entry, icons, and documentation.
#
# Environment Variables:
#   PREFIX   Installation prefix (default: /usr/local)
#   DESTDIR  Staging directory prefix (default: empty)
#
# Options:
#   --uninstall, -u   Remove all installed Tilo files
#   --help, -h        Show usage information
# ==============================================================================

set -eu

BASE_DIR="$(cd "$(dirname "$0")" && pwd)"
NAME="tilo"
PREFIX="${PREFIX:-/usr/local}"
DESTDIR="${DESTDIR:-}"

TARGET_BIN_DIR="${DESTDIR}${PREFIX}/bin"
TARGET_DATA_DIR="${DESTDIR}${PREFIX}/share"
TARGET_XSESSION_DIR="${TARGET_DATA_DIR}/xsessions"
TARGET_APPS_DIR="${TARGET_DATA_DIR}/applications"
TARGET_ICONS_DIR="${TARGET_DATA_DIR}/icons/hicolor"
TARGET_PIXMAPS_DIR="${TARGET_DATA_DIR}/pixmaps"
TARGET_SLICK_DIR="${TARGET_DATA_DIR}/slick-greeter/badges"
TARGET_DOC_DIR="${TARGET_DATA_DIR}/doc/${NAME}"

usage() {
    cat << EOF
Usage: $0 [options]

Options:
  -u, --uninstall   Uninstall Tilo from ${PREFIX}
  -h, --help        Show this help message

Environment Variables:
  PREFIX            Installation directory prefix (default: /usr/local)
  DESTDIR           Staged install directory (default: empty)

Examples:
  sudo ./install.sh                     # System-wide install (/usr/local)
  PREFIX=~/.local ./install.sh          # User-local install (~/.local)
  sudo ./install.sh --uninstall         # Remove installed files
EOF
    exit 0
}

# Parse options
for arg in "$@"; do
    case "$arg" in
        -u|--uninstall)
            echo "==> Uninstalling ${NAME} from ${DESTDIR}${PREFIX}..."
            rm -f "${TARGET_BIN_DIR}/${NAME}"
            rm -f "${TARGET_XSESSION_DIR}/${NAME}.desktop"
            rm -f "${TARGET_APPS_DIR}/${NAME}.desktop"
            rm -f "${TARGET_ICONS_DIR}/scalable/apps/${NAME}.svg"
            rm -f "${TARGET_ICONS_DIR}/scalable/apps/${NAME}-badge-symbolic.svg"
            rm -f "${TARGET_ICONS_DIR}/scalable/apps/${NAME}_badge-symbolic.svg"
            rm -f "${TARGET_ICONS_DIR}/scalable/apps/${NAME}-symbolic.svg"
            rm -f "${TARGET_ICONS_DIR}/scalable/badges/${NAME}.svg"
            rm -f "${TARGET_ICONS_DIR}/scalable/badges/${NAME}-symbolic.svg"
            rm -f "${TARGET_SLICK_DIR}/${NAME}.svg"
            rm -f "${TARGET_PIXMAPS_DIR}/${NAME}.svg"
            rm -f "${TARGET_PIXMAPS_DIR}/${NAME}.png"
            rm -rf "${TARGET_DOC_DIR}"

            # Update desktop & icon caches if available
            if command -v update-desktop-database >/dev/null 2>&1; then
                update-desktop-database "${TARGET_APPS_DIR}" 2>/dev/null || true
            fi
            if command -v gtk-update-icon-cache >/dev/null 2>&1; then
                gtk-update-icon-cache -q -t "${TARGET_ICONS_DIR}" 2>/dev/null || true
            fi

            echo "==> ${NAME} successfully uninstalled."
            exit 0
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "Unknown option: $arg" >&2
            usage
            ;;
    esac
done

# Locate binary
BIN_SRC=""
if [ -f "${BASE_DIR}/${NAME}" ]; then
    BIN_SRC="${BASE_DIR}/${NAME}"
elif [ -f "${BASE_DIR}/bin/${NAME}" ]; then
    BIN_SRC="${BASE_DIR}/bin/${NAME}"
else
    echo "Error: Cannot find '${NAME}' executable in ${BASE_DIR}!" >&2
    exit 1
fi

ASSETS_DIR="${BASE_DIR}/assets"
if [ ! -d "${ASSETS_DIR}" ]; then
    echo "Error: Assets directory not found at ${ASSETS_DIR}!" >&2
    exit 1
fi

echo "==> Installing ${NAME} to ${DESTDIR}${PREFIX}..."

# Check write permissions
mkdir -p "${TARGET_BIN_DIR}" 2>/dev/null || {
    echo "Error: Permission denied writing to '${TARGET_BIN_DIR}'." >&2
    echo "Please run with sudo: 'sudo ./install.sh'" >&2
    echo "Or specify a user directory: 'PREFIX=~/.local ./install.sh'" >&2
    exit 1
}

# 1. Install executable
mkdir -p "${TARGET_BIN_DIR}"
install -m 755 "${BIN_SRC}" "${TARGET_BIN_DIR}/${NAME}"
echo "  -> Installed ${TARGET_BIN_DIR}/${NAME}"

# 2. Install desktop entries
mkdir -p "${TARGET_XSESSION_DIR}" "${TARGET_APPS_DIR}"
if [ -f "${ASSETS_DIR}/${NAME}.desktop" ]; then
    install -m 644 "${ASSETS_DIR}/${NAME}.desktop" "${TARGET_XSESSION_DIR}/${NAME}.desktop"
    install -m 644 "${ASSETS_DIR}/${NAME}.desktop" "${TARGET_APPS_DIR}/${NAME}.desktop"
    echo "  -> Installed XSession desktop entry: ${TARGET_XSESSION_DIR}/${NAME}.desktop"
fi

# 3. Install icons & badges
mkdir -p "${TARGET_ICONS_DIR}/scalable/apps" "${TARGET_ICONS_DIR}/scalable/badges"
mkdir -p "${TARGET_SLICK_DIR}" "${TARGET_PIXMAPS_DIR}"

if [ -f "${ASSETS_DIR}/svg/${NAME}.svg" ]; then
    install -m 644 "${ASSETS_DIR}/svg/${NAME}.svg" "${TARGET_ICONS_DIR}/scalable/apps/${NAME}.svg"
    install -m 644 "${ASSETS_DIR}/svg/${NAME}.svg" "${TARGET_ICONS_DIR}/scalable/apps/${NAME}-badge-symbolic.svg"
    install -m 644 "${ASSETS_DIR}/svg/${NAME}.svg" "${TARGET_ICONS_DIR}/scalable/apps/${NAME}_badge-symbolic.svg"
    install -m 644 "${ASSETS_DIR}/svg/${NAME}.svg" "${TARGET_ICONS_DIR}/scalable/apps/${NAME}-symbolic.svg"
    install -m 644 "${ASSETS_DIR}/svg/${NAME}.svg" "${TARGET_ICONS_DIR}/scalable/badges/${NAME}.svg"
    install -m 644 "${ASSETS_DIR}/svg/${NAME}.svg" "${TARGET_ICONS_DIR}/scalable/badges/${NAME}-symbolic.svg"
    install -m 644 "${ASSETS_DIR}/svg/${NAME}.svg" "${TARGET_SLICK_DIR}/${NAME}.svg"
    install -m 644 "${ASSETS_DIR}/svg/${NAME}.svg" "${TARGET_PIXMAPS_DIR}/${NAME}.svg"
fi

if [ -f "${ASSETS_DIR}/png/tilo_dark_transparent.png" ]; then
    install -m 644 "${ASSETS_DIR}/png/tilo_dark_transparent.png" "${TARGET_PIXMAPS_DIR}/${NAME}.png"
fi
echo "  -> Installed desktop icons and session badges"

# 4. Install documentation
mkdir -p "${TARGET_DOC_DIR}"
if [ -f "${BASE_DIR}/README.md" ]; then
    install -m 644 "${BASE_DIR}/README.md" "${TARGET_DOC_DIR}/"
fi
if [ -f "${BASE_DIR}/LICENSE" ]; then
    install -m 644 "${BASE_DIR}/LICENSE" "${TARGET_DOC_DIR}/"
fi
if [ -f "${BASE_DIR}/GUIDE.md" ]; then
    install -m 644 "${BASE_DIR}/GUIDE.md" "${TARGET_DOC_DIR}/"
fi
echo "  -> Installed documentation to ${TARGET_DOC_DIR}"

# 5. Update system caches
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${TARGET_APPS_DIR}" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t "${TARGET_ICONS_DIR}" 2>/dev/null || true
fi

echo ""
echo "==> ${NAME} successfully installed to ${DESTDIR}${PREFIX}!"
echo "    - Run 'tilo' to start or select Tilo in your display manager."
echo "    - To uninstall, run: $0 --uninstall"
