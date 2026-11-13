#!/usr/bin/env bash
# ==============================================================================
# pkg-rpm.sh - Fedora / RHEL Package Builder for Tilo
# ==============================================================================
# Prepares RPM sources, spec file, and builds RPM packages using rpmbuild
# (or optionally via a Fedora Docker container if rpmbuild is not installed locally).
#
# Usage:
#   ./scripts/packaging/fedora/pkg-rpm.sh                      # Native source build (requires rpmbuild)
#   ./scripts/packaging/fedora/pkg-rpm.sh --docker             # Containerized build via Fedora image
#   ./scripts/packaging/fedora/pkg-rpm.sh --binary <path> --arch <arch> [--out-dir <dir>]
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
DIST_DIR="${REPO_ROOT}/dist"

cd "${REPO_ROOT}"

# Parse options
CUSTOM_BIN=""
OVERRIDE_ARCH=""
OUT_DIR="${DIST_DIR}"
USE_DOCKER=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --binary|-b)
            CUSTOM_BIN="$2"
            shift 2
            ;;
        --arch|-a)
            OVERRIDE_ARCH="$2"
            shift 2
            ;;
        --out-dir|-o)
            OUT_DIR="$2"
            shift 2
            ;;
        --docker)
            USE_DOCKER=true
            shift
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

# Resolve absolute paths
OUT_DIR="$(mkdir -p "${OUT_DIR}" && cd "${OUT_DIR}" && pwd)"
if [[ -n "${CUSTOM_BIN}" ]]; then
    if [[ ! -f "${CUSTOM_BIN}" ]]; then
        echo "Error: Specified binary '${CUSTOM_BIN}' not found!" >&2
        exit 1
    fi
    CUSTOM_BIN="$(cd "$(dirname "${CUSTOM_BIN}")" && pwd)/$(basename "${CUSTOM_BIN}")"
fi

# Read current version from VERSION file
VERSION="$(tr -d '[:space:]' < "${REPO_ROOT}/VERSION")"
RPM_TOPDIR="${OUT_DIR}/rpmbuild"

# Detect architecture
if [[ -n "${OVERRIDE_ARCH}" ]]; then
    RPM_ARCH="${OVERRIDE_ARCH}"
else
    RAW_ARCH="$(uname -m)"
    case "${RAW_ARCH}" in
        x86_64)  RPM_ARCH="x86_64" ;;
        aarch64) RPM_ARCH="aarch64" ;;
        armv7l)  RPM_ARCH="armv7hl" ;;
        i686)    RPM_ARCH="i686" ;;
        *)       RPM_ARCH="${RAW_ARCH}" ;;
    esac
fi

echo "==> Preparing Fedora RPM packaging for tilo v${VERSION} (${RPM_ARCH})..."

mkdir -p "${OUT_DIR}"
mkdir -p "${RPM_TOPDIR}"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}

# Synchronize Version in tilo.spec
sed -i -E "s/^Version:.*/Version:        ${VERSION}/" "${SCRIPT_DIR}/tilo.spec"
cp "${SCRIPT_DIR}/tilo.spec" "${RPM_TOPDIR}/SPECS/tilo.spec"

# Fast path: Pre-built binary packaging
if [[ -n "${CUSTOM_BIN}" ]]; then
    if [[ ! -f "${CUSTOM_BIN}" ]]; then
        echo "Error: Specified binary '${CUSTOM_BIN}' not found!" >&2
        exit 1
    fi

    STAGING_DIR="${OUT_DIR}/rpm-staging"
    rm -rf "${STAGING_DIR}"
    mkdir -p "${STAGING_DIR}"
    make install DESTDIR="${STAGING_DIR}" PREFIX=/usr BIN_SRC="${CUSTOM_BIN}"

    if command -v rpmbuild >/dev/null 2>&1; then
        echo "==> Running rpmbuild on binary package..."
        rpmbuild \
            --define "version ${VERSION}" \
            --define "target_arch ${RPM_ARCH}" \
            --define "staging_dir ${STAGING_DIR}" \
            --define "_topdir ${RPM_TOPDIR}" \
            --target "${RPM_ARCH}" \
            -bb "${SCRIPT_DIR}/tilo-bin.spec"

        find "${RPM_TOPDIR}/RPMS" -type f -name "*.rpm" -exec cp {} "${OUT_DIR}/" \;
        rm -rf "${STAGING_DIR}"
        echo "==> Binary RPM build complete in ${OUT_DIR}:"
        ls -lh "${OUT_DIR}"/*.rpm || true
        exit 0
    else
        echo "==> Error: 'rpmbuild' is required for binary RPM packaging." >&2
        exit 1
    fi
fi

# Source tarball packaging path
TARBALL_NAME="tilo-${VERSION}.tar.gz"
echo "==> Creating source archive ${TARBALL_NAME} in SOURCES/..."

if git rev-parse --is-inside-work-tree >/dev/null 2>&1 && git diff --quiet HEAD 2>/dev/null && [ -z "$(git status --porcelain 2>/dev/null)" ]; then
    # Archive clean git tree with version prefix
    git archive --format=tar.gz --prefix="tilo-${VERSION}/" HEAD > "${RPM_TOPDIR}/SOURCES/${TARBALL_NAME}"
else
    # Include working directory files (uncommitted / untracked assets)
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
    find "${RPM_TOPDIR}/RPMS" -type f -name "*.rpm" -exec cp {} "${OUT_DIR}/" \;
    echo "==> Docker RPM build complete:"
    ls -lh "${OUT_DIR}"/*.rpm
    exit 0
fi

if command -v rpmbuild >/dev/null 2>&1; then
    echo "==> Running native rpmbuild..."
    rpmbuild --define "_topdir ${RPM_TOPDIR}" -bb "${RPM_TOPDIR}/SPECS/tilo.spec"
    
    # Copy generated RPMs to output dir
    find "${RPM_TOPDIR}/RPMS" -type f -name "*.rpm" -exec cp {} "${OUT_DIR}/" \;
    echo "==> RPM build complete:"
    ls -lh "${OUT_DIR}"/*.rpm
else
    echo "==> 'rpmbuild' is not installed on this system."
    echo "==> RPM workspace and source archive are ready in:"
    echo "      ${RPM_TOPDIR}/"
    echo "      - Spec file:    ${RPM_TOPDIR}/SPECS/tilo.spec"
    echo "      - Source archive: ${RPM_TOPDIR}/SOURCES/${TARBALL_NAME}"
fi
