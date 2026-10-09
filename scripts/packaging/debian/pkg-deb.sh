#!/usr/bin/env bash
# ==============================================================================
# pkg-deb.sh - Debian / Ubuntu Package Builder for Tilo
# ==============================================================================
# Builds a Debian package (.deb) using dpkg-deb (or standalone ar/tar fallback).
# Output: dist/tilo_<version>_<arch>.deb
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
DIST_DIR="${REPO_ROOT}/dist"

cd "${REPO_ROOT}"

# Read current version from VERSION file
VERSION="$(tr -d '[:space:]' < "${REPO_ROOT}/VERSION")"

# Detect Debian architecture
if command -v dpkg >/dev/null 2>&1 && dpkg --print-architecture >/dev/null 2>&1; then
    DEB_ARCH="$(dpkg --print-architecture)"
else
    RAW_ARCH="$(uname -m)"
    case "${RAW_ARCH}" in
        x86_64)  DEB_ARCH="amd64" ;;
        aarch64) DEB_ARCH="arm64" ;;
        armv7l)  DEB_ARCH="armhf" ;;
        i686)    DEB_ARCH="i386" ;;
        *)       DEB_ARCH="${RAW_ARCH}" ;;
    esac
fi

echo "==> Preparing Debian package for tilo v${VERSION} (${DEB_ARCH})..."

mkdir -p "${DIST_DIR}"

# Build release executable
make release

STAGING_DIR="${DIST_DIR}/debian-staging"
rm -rf "${STAGING_DIR}"
mkdir -p "${STAGING_DIR}/DEBIAN"

# Install files using Makefile GNU paths
make install DESTDIR="${STAGING_DIR}" PREFIX=/usr

# Calculate installed size in KB
INSTALLED_SIZE="$(du -ks "${STAGING_DIR}" | cut -f1)"

# Generate DEBIAN/control
cat <<EOF > "${STAGING_DIR}/DEBIAN/control"
Package: tilo
Version: ${VERSION}
Section: x11
Priority: optional
Architecture: ${DEB_ARCH}
Maintainer: Tilo Developers <dev@tilo.org>
Installed-Size: ${INSTALLED_SIZE}
Depends: libc6, libx11-6, libxrandr2, libxcb1
Recommends: fonts-dejavu-core | fonts-liberation
Suggests: xserver-xephyr
Description: Minimal Rust X11 Tiling Window Manager
 Tilo is a standalone, lightweight, fast X11 tiling window manager written
 in Rust. Features include master-stack, monocle, bsp, and tabbed layouts,
 an integrated status bar and launcher, workspace management, and ICCCM/EWMH
 compliance.
EOF

# Generate DEBIAN/md5sums
(
    cd "${STAGING_DIR}"
    find . -type f ! -path './DEBIAN/*' -print0 | sort -z | xargs -0 md5sum | sed -e 's| \./| |' > DEBIAN/md5sums
)

# Set standard permissions
chmod 755 "${STAGING_DIR}/DEBIAN"
chmod 644 "${STAGING_DIR}/DEBIAN/control" "${STAGING_DIR}/DEBIAN/md5sums"

DEB_FILE="${DIST_DIR}/tilo_${VERSION}_${DEB_ARCH}.deb"

if command -v dpkg-deb >/dev/null 2>&1; then
    echo "==> Building .deb package using dpkg-deb..."
    dpkg-deb --build --root-owner-group "${STAGING_DIR}" "${DEB_FILE}"
else
    echo "==> dpkg-deb not found. Creating deb package archive manually..."
    # Format: debian-binary, control.tar.gz, data.tar.gz in an ar archive
    echo "2.0" > "${STAGING_DIR}/debian-binary"
    (
        cd "${STAGING_DIR}/DEBIAN"
        tar --numeric-owner --owner=0 --group=0 -czf "${STAGING_DIR}/control.tar.gz" control md5sums
    )
    (
        cd "${STAGING_DIR}"
        tar --numeric-owner --owner=0 --group=0 -czf "${STAGING_DIR}/data.tar.gz" usr
    )
    ar rcs "${DEB_FILE}" "${STAGING_DIR}/debian-binary" "${STAGING_DIR}/control.tar.gz" "${STAGING_DIR}/data.tar.gz"
fi

rm -rf "${STAGING_DIR}"

echo "==> Debian package built successfully: ${DEB_FILE}"
if command -v dpkg-deb >/dev/null 2>&1; then
    echo "==> Package information:"
    dpkg-deb -I "${DEB_FILE}"
fi
