#!/usr/bin/env bash
# ==============================================================================
# pkg-bin.sh - Standalone Binary Archive (.tar.gz) Builder for Tilo
# ==============================================================================
# Packages the Tilo executable, self-contained install.sh installer, multi-distro
# GUIDE.md, desktop entries, icons, and documentation into a distribution tarball.
#
# Options:
#   --binary, -b <path>   Use pre-built binary instead of compiling natively
#   --target, -t <target> Target triple or architecture tag (default: uname -m)
#   --out-dir, -o <dir>   Output directory (default: dist/)
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
DIST_DIR="${REPO_ROOT}/dist"

cd "${REPO_ROOT}"

CUSTOM_BIN=""
TARGET_NAME=""
OUT_DIR="${DIST_DIR}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --binary|-b)
            CUSTOM_BIN="$2"
            shift 2
            ;;
        --target|-t)
            TARGET_NAME="$2"
            shift 2
            ;;
        --out-dir|-o)
            OUT_DIR="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

# Resolve absolute output directory
OUT_DIR="$(mkdir -p "${OUT_DIR}" && cd "${OUT_DIR}" && pwd)"

# Resolve binary path
if [[ -n "${CUSTOM_BIN}" ]]; then
    if [[ ! -f "${CUSTOM_BIN}" ]]; then
        echo "Error: Specified binary '${CUSTOM_BIN}' not found!" >&2
        exit 1
    fi
    CUSTOM_BIN="$(cd "$(dirname "${CUSTOM_BIN}")" && pwd)/$(basename "${CUSTOM_BIN}")"
fi

# Read version
VERSION="$(tr -d '[:space:]' < "${REPO_ROOT}/VERSION")"

# Determine target identifier
if [[ -z "${TARGET_NAME}" ]]; then
    TARGET_NAME="$(uname -m)"
fi

echo "==> Preparing standalone binary archive for tilo v${VERSION} (${TARGET_NAME})..."

# Ensure binary is built if not provided
if [[ -z "${CUSTOM_BIN}" ]]; then
    make release
    CUSTOM_BIN="${REPO_ROOT}/target/release/tilo"
fi

ARCHIVE_NAME="tilo-v${VERSION}-${TARGET_NAME}"
STAGING_DIR="${OUT_DIR}/${ARCHIVE_NAME}"

rm -rf "${STAGING_DIR}"
mkdir -p "${STAGING_DIR}"

# 1. Copy binary
install -m 755 "${CUSTOM_BIN}" "${STAGING_DIR}/tilo"

# 2. Copy standalone installer
install -m 755 "${SCRIPT_DIR}/assets/install.sh" "${STAGING_DIR}/install.sh"

# 3. Copy documentation & guides
install -m 644 "${SCRIPT_DIR}/assets/GUIDE.md" "${STAGING_DIR}/GUIDE.md"
if [ -f "${REPO_ROOT}/README.md" ]; then
    install -m 644 "${REPO_ROOT}/README.md" "${STAGING_DIR}/README.md"
fi
if [ -f "${REPO_ROOT}/LICENSE" ]; then
    install -m 644 "${REPO_ROOT}/LICENSE" "${STAGING_DIR}/LICENSE"
fi

# 4. Copy desktop assets (desktop entry, SVG/PNG icons, badges)
mkdir -p "${STAGING_DIR}/assets"
cp -r "${REPO_ROOT}/assets/"* "${STAGING_DIR}/assets/"

# 5. Create tar.gz archive
TARBALL_FILE="${OUT_DIR}/${ARCHIVE_NAME}.tar.gz"
echo "==> Compressing into ${TARBALL_FILE}..."
(
    cd "${OUT_DIR}"
    tar --numeric-owner --owner=0 --group=0 -czf "${TARBALL_FILE}" "${ARCHIVE_NAME}"
    sha256sum "$(basename "${TARBALL_FILE}")" > "${TARBALL_FILE}.sha256"
)

# Cleanup staging directory
rm -rf "${STAGING_DIR}"

echo "==> Standalone binary archive created successfully:"
ls -lh "${TARBALL_FILE}"
cat "${TARBALL_FILE}.sha256"
