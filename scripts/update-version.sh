#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

VERSION_FILE="${ROOT_DIR}/VERSION"
CARGO_TOML="${ROOT_DIR}/Cargo.toml"

CURRENT_VERSION="$(tr -d '[:space:]' < "${VERSION_FILE}")"

usage() {
    echo "Usage: $0 [<new_version> | patch | minor | major | check]"
    echo ""
    echo "Options:"
    echo "  <new_version>   Explicit semver (e.g. 0.2.0)"
    echo "  patch           Bump patch version (e.g. 0.1.0 -> 0.1.1)"
    echo "  minor           Bump minor version (e.g. 0.1.0 -> 0.2.0)"
    echo "  major           Bump major version (e.g. 0.1.0 -> 1.0.0)"
    echo "  check           Verify VERSION and Cargo.toml are in sync"
    exit 1
}

if [[ $# -eq 0 ]]; then
    echo "Current version: ${CURRENT_VERSION}"
    usage
fi

ACTION="$1"

if [[ "${ACTION}" == "check" ]]; then
    CARGO_VER="$(grep -m1 '^version = ' "${CARGO_TOML}" | cut -d '"' -f2)"
    echo "VERSION file:  ${CURRENT_VERSION}"
    echo "Cargo.toml:    ${CARGO_VER}"
    if [[ "${CURRENT_VERSION}" == "${CARGO_VER}" ]]; then
        echo "Versions match: OK"
        exit 0
    else
        echo "Error: Version mismatch detected!" >&2
        exit 1
    fi
fi

# Parse current version into components
IFS='.' read -r MAJOR MINOR PATCH <<< "${CURRENT_VERSION}"

case "${ACTION}" in
    patch)
        PATCH=$((PATCH + 1))
        NEW_VERSION="${MAJOR}.${MINOR}.${PATCH}"
        ;;
    minor)
        MINOR=$((MINOR + 1))
        PATCH=0
        NEW_VERSION="${MAJOR}.${MINOR}.${PATCH}"
        ;;
    major)
        MAJOR=$((MAJOR + 1))
        MINOR=0
        PATCH=0
        NEW_VERSION="${MAJOR}.${MINOR}.${PATCH}"
        ;;
    *)
        # Validate semver format X.Y.Z
        if [[ ! "${ACTION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
            echo "Error: Invalid semver format: ${ACTION}" >&2
            exit 1
        fi
        NEW_VERSION="${ACTION}"
        ;;
esac

echo "Updating version: ${CURRENT_VERSION} -> ${NEW_VERSION}"

# Update VERSION file
echo "${NEW_VERSION}" > "${VERSION_FILE}"

# Update Cargo.toml (only the package version line)
sed -i -E "s/^(version[[:space:]]*=[[:space:]]*\")[^\"]+(\".*)/\1${NEW_VERSION}\2/" "${CARGO_TOML}"

# Update Cargo.lock to match
if command -v cargo >/dev/null 2>&1; then
    (cd "${ROOT_DIR}" && cargo check --quiet)
fi

echo "Successfully updated version to ${NEW_VERSION} in VERSION, Cargo.toml, and Cargo.lock"
