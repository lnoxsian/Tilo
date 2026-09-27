#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Configuration defaults
SCREEN_SIZE="1280x720"
BUILD_MODE="debug"
SPAWN_CLIENTS=0
XEPHYR_BIN="$(command -v Xephyr || true)"

if [[ -z "${XEPHYR_BIN}" ]]; then
    echo "Error: Xephyr is not installed on this system." >&2
    echo "Install it via your package manager (e.g. apt install xserver-xephyr)." >&2
    exit 1
fi

usage() {
    echo "Usage: $0 [options]"
    echo ""
    echo "Options:"
    echo "  --release          Run release build of tilo instead of debug"
    echo "  --screen <WIDTHxHEIGHT>  Set Xephyr screen resolution (default: 1280x720)"
    echo "  --with-clients     Launch sample X11 apps (xclock, xeyes) inside Xephyr"
    echo "  -h, --help         Show this help message"
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release)
            BUILD_MODE="release"
            shift
            ;;
        --screen)
            SCREEN_SIZE="$2"
            shift 2
            ;;
        --with-clients)
            SPAWN_CLIENTS=1
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "Unknown option: $1" >&2
            usage
            ;;
    esac
done

# Find available X display
XEPHYR_DISPLAY=""
for i in {1..99}; do
    if [[ ! -e "/tmp/.X11-unix/X${i}" && ! -e "/tmp/.X${i}-lock" ]]; then
        XEPHYR_DISPLAY=":${i}"
        break
    fi
done

if [[ -z "${XEPHYR_DISPLAY}" ]]; then
    echo "Error: Could not find an available X display for Xephyr." >&2
    exit 1
fi

echo "=========================================="
echo " Starting Xephyr on display ${XEPHYR_DISPLAY}"
echo " Resolution: ${SCREEN_SIZE}"
echo " Build mode: ${BUILD_MODE}"
echo "=========================================="

# Build tilo before launching Xephyr
if [[ "${BUILD_MODE}" == "release" ]]; then
    (cd "${ROOT_DIR}" && cargo build --release)
    TILO_BIN="${ROOT_DIR}/target/release/tilo"
else
    (cd "${ROOT_DIR}" && cargo build)
    TILO_BIN="${ROOT_DIR}/target/debug/tilo"
fi

XEPHYR_PID=""
CLIENT_PIDS=()

cleanup() {
    echo ""
    echo "Cleaning up Xephyr session..."
    for pid in "${CLIENT_PIDS[@]}"; do
        if kill -0 "${pid}" 2>/dev/null; then
            kill "${pid}" 2>/dev/null || true
        fi
    done
    if [[ -n "${XEPHYR_PID}" ]] && kill -0 "${XEPHYR_PID}" 2>/dev/null; then
        kill "${XEPHYR_PID}" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

# Start Xephyr
"${XEPHYR_BIN}" "${XEPHYR_DISPLAY}" \
    -ac \
    -screen "${SCREEN_SIZE}" \
    -br \
    -reset \
    -terminate &
XEPHYR_PID=$!

# Wait for Xephyr display to be ready
echo "Waiting for Xephyr X server to become ready..."
READY=0
for _ in {1..30}; do
    if DISPLAY="${XEPHYR_DISPLAY}" xset q >/dev/null 2>&1 || [[ -e "/tmp/.X11-unix/X${XEPHYR_DISPLAY#:}" ]]; then
        READY=1
        break
    fi
    sleep 0.1
done

if [[ ${READY} -eq 0 ]]; then
    echo "Warning: Display socket wait timed out, continuing anyway..."
else
    echo "Xephyr is ready."
fi

# Optionally launch sample X11 clients
if [[ ${SPAWN_CLIENTS} -eq 1 ]]; then
    if command -v xclock >/dev/null 2>&1; then
        DISPLAY="${XEPHYR_DISPLAY}" xclock &
        CLIENT_PIDS+=($!)
    fi
    if command -v xeyes >/dev/null 2>&1; then
        DISPLAY="${XEPHYR_DISPLAY}" xeyes &
        CLIENT_PIDS+=($!)
    fi
fi

# Run tilo on the Xephyr display
echo "Launching tilo on ${XEPHYR_DISPLAY}..."
DISPLAY="${XEPHYR_DISPLAY}" "${TILO_BIN}" || true

# Wait for Xephyr process if tilo exited immediately
wait "${XEPHYR_PID}" 2>/dev/null || true
