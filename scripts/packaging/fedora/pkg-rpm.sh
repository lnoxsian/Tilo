#!/usr/bin/env bash
# ==============================================================================
# pkg-rpm.sh - Fedora / RHEL Package Builder for Tilo
# ==============================================================================
# Prepares RPM sources, spec file, and builds RPM packages using rpmbuild
# (or optionally via a Fedora Docker container if rpmbuild is not installed locally).
#
# Usage:
#   ./scripts/packaging/fedora/pkg-rpm.sh          # Native build (requires rpmbuild)
#   ./scripts/packaging/fedora/pkg-rpm.sh --docker # Containerized build via Fedora image
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
DIST_DIR="${REPO_ROOT}/dist"
RPM_TOPDIR="${DIST_DIR}/rpmbuild"

cd "${REPO_ROOT}"

# Read current version from VERSION file
VERSION="$(tr -d '[:space:]' < "${REPO_ROOT}/VERSION")"
USE_DOCKER=false

for arg in "$@"; do
    case "$arg" in
        --docker) USE_DOCKER=true ;;
    esac
done

echo "==> Preparing Fedora RPM packaging for tilo v${VERSION}..."

mkdir -p "${DIST_DIR}"
mkdir -p "${RPM_TOPDIR}"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}

# Synchronize Version in tilo.spec
sed -i -E "s/^Version:.*/Version:        ${VERSION}/" "${SCRIPT_DIR}/tilo.spec"
cp "${SCRIPT_DIR}/tilo.spec" "${RPM_TOPDIR}/SPECS/tilo.spec"

# Create clean source tarball in SOURCES/
TARBALL_NAME="tilo-${VERSION}.tar.gz"
echo "==> Creating source archive ${TARBALL_NAME} in SOURCES/..."

if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    # Archive git tree with version prefix
    git archive --format=tar.gz --prefix="tilo-${VERSION}/" HEAD > "${RPM_TOPDIR}/SOURCES/${TARBALL_NAME}"
else
    tar --exclude='./target' --exclude='./dist' --exclude='./.git' \
        --transform="s,^\.,tilo-${VERSION}," -czf "${RPM_TOPDIR}/SOURCES/${TARBALL_NAME}" .
fi

if [ "${USE_DOCKER}" = true ]; then
    echo "==> Building RPM inside Fedora container..."
    docker run --rm \
        -v "${REPO_ROOT}:/workspace:z" \
        -w /workspace \
        fedora:latest bash -c "
            dnf install -y rpm-build cargo make gcc libX11-devel libXrandr-devel libxcb-devel && \
            rpmbuild --define '_topdir /workspace/dist/rpmbuild' -bb /workspace/dist/rpmbuild/SPECS/tilo.spec
        "
    find "${RPM_TOPDIR}/RPMS" -type f -name "*.rpm" -exec cp {} "${DIST_DIR}/" \;
    echo "==> Docker RPM build complete:"
    ls -lh "${DIST_DIR}"/*.rpm
    exit 0
fi

if command -v rpmbuild >/dev/null 2>&1; then
    echo "==> Running native rpmbuild..."
    rpmbuild --define "_topdir ${RPM_TOPDIR}" -bb "${RPM_TOPDIR}/SPECS/tilo.spec"
    
    # Copy generated RPMs to dist/
    find "${RPM_TOPDIR}/RPMS" -type f -name "*.rpm" -exec cp {} "${DIST_DIR}/" \;
    echo "==> RPM build complete:"
    ls -lh "${DIST_DIR}"/*.rpm
else
    echo "==> 'rpmbuild' is not installed on this system."
    echo "==> RPM workspace and source archive are ready in:"
    echo "      ${RPM_TOPDIR}/"
    echo "      - Spec file:    ${RPM_TOPDIR}/SPECS/tilo.spec"
    echo "      - Source archive: ${RPM_TOPDIR}/SOURCES/${TARBALL_NAME}"
    echo ""
    echo "To build the RPM, you can either:"
    echo "  1. Run with Docker (if installed):"
    echo "     ./scripts/packaging/fedora/pkg-rpm.sh --docker"
    echo "  2. Run on a Fedora / RHEL system:"
    echo "     sudo dnf install rpm-build cargo make gcc libX11-devel libXrandr-devel libxcb-devel"
    echo "     rpmbuild --define '_topdir \$(pwd)/dist/rpmbuild' -bb dist/rpmbuild/SPECS/tilo.spec"
    echo "  3. On Debian/Ubuntu host, install rpm-build tools:"
    echo "     sudo apt install rpm"
fi
