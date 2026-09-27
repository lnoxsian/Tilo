#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Configuration defaults
SCREEN_SIZE="1280x720"
BUILD_MODE="debug"
SPAWN_CLIENTS=0
LAYOUT_DEMO=0
LAYOUT_TYPE="all"
AUTOMATED_TEST=0
XEPHYR_BIN="$(command -v Xephyr || true)"

usage() {
    echo "Usage: $0 [options]"
    echo ""
    echo "Options:"
    echo "  --test-layout [type]   Demo implemented layouts in Xephyr (types: single, master, stack, floating, all; default: all)"
    echo "  --layout-demo [type]   Alias for --test-layout"
    echo "  --automated-tests      Run non-interactive cargo layout test suite"
    echo "  --release              Run release build of tilo instead of debug"
    echo "  --screen <WxH>         Set Xephyr screen resolution (default: 1280x720)"
    echo "  --with-clients         Launch sample X11 apps inside Xephyr"
    echo "  -h, --help             Show this help message"
    echo ""
    echo "Layout types for --test-layout:"
    echo "  single    - Test 1-window fullscreen tiling layout (xclock)"
    echo "  master    - Test 2-window Master/Stack split (xclock + xcalc)"
    echo "  stack     - Test 3-window Master + Vertical Stack (xclock + xcalc + xeyes)"
    echo "  floating  - Test Tiled & Floating window layering (Mod4+Space toggle)"
    echo "  all       - Test all implemented layout features (default)"
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --test-layout|--layout-demo|--layout|--test-layouts)
            LAYOUT_DEMO=1
            if [[ $# -ge 2 ]] && [[ ! "$2" =~ ^- ]]; then
                LAYOUT_TYPE="$2"
                shift 2
            else
                LAYOUT_TYPE="all"
                shift 1
            fi
            ;;
        --automated-tests|--test-cargo)
            AUTOMATED_TEST=1
            shift
            ;;
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

# If non-interactive automated test mode requested
if [[ ${AUTOMATED_TEST} -eq 1 ]]; then
    echo "============================================================"
    echo " Running Layout Integration Tests in Xephyr"
    echo " Implemented Layouts: Master/Stack Layout & Floating Layout"
    echo "============================================================"
    cd "${ROOT_DIR}"
    cargo test --test layout --test floating -- --nocapture
    echo ""
    echo "==> All implemented layout tests passed successfully!"
    exit 0
fi

if [[ -z "${XEPHYR_BIN}" ]]; then
    echo "Error: Xephyr is not installed on this system." >&2
    echo "Install it via your package manager (e.g. apt install xserver-xephyr)." >&2
    exit 1
fi

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
if [[ ${LAYOUT_DEMO} -eq 1 ]]; then
    echo " Layout Demo: ${LAYOUT_TYPE}"
fi
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
    -noreset &
XEPHYR_PID=$!

# Wait for Xephyr display socket to become ready
echo "Waiting for Xephyr X server to become ready..."
READY=0
for _ in {1..50}; do
    if [[ -S "/tmp/.X11-unix/X${XEPHYR_DISPLAY#:}" ]] || [[ -e "/tmp/.X11-unix/X${XEPHYR_DISPLAY#:}" ]]; then
        READY=1
        break
    fi
    sleep 0.05
done

if [[ ${READY} -eq 0 ]]; then
    echo "Warning: Display socket wait timed out, continuing anyway..."
else
    echo "Xephyr is ready."
fi

if [[ ${LAYOUT_DEMO} -eq 1 ]]; then
    echo "Launching tilo on ${XEPHYR_DISPLAY}..."
    DISPLAY="${XEPHYR_DISPLAY}" "${TILO_BIN}" &
    TILO_PID=$!
    CLIENT_PIDS+=("${TILO_PID}")

    # Give tilo a moment to initialize and claim WM ownership
    sleep 0.4

    case "${LAYOUT_TYPE}" in
        single)
            echo "Spawning single window to demo Single-Window Fullscreen tiling..."
            if command -v xclock >/dev/null 2>&1; then
                echo " -> Spawning Window: xclock (Full usable screen area)"
                DISPLAY="${XEPHYR_DISPLAY}" xclock &
                CLIENT_PIDS+=($!)
            fi
            ;;

        master|split|two)
            echo "Spawning 2 windows to demo Master/Stack split layout..."
            if command -v xclock >/dev/null 2>&1; then
                echo " -> Spawning Window 1: xclock (Master pane: left 55%)"
                DISPLAY="${XEPHYR_DISPLAY}" xclock &
                CLIENT_PIDS+=($!)
                sleep 0.5
            fi
            if command -v xcalc >/dev/null 2>&1; then
                echo " -> Spawning Window 2: xcalc (Stack pane: right 45%)"
                DISPLAY="${XEPHYR_DISPLAY}" xcalc &
                CLIENT_PIDS+=($!)
            fi
            ;;

        stack|multi|three)
            echo "Spawning 3 windows to demo Master + Vertical Stack layout..."
            if command -v xclock >/dev/null 2>&1; then
                echo " -> Spawning Window 1: xclock (Master pane: left 55%)"
                DISPLAY="${XEPHYR_DISPLAY}" xclock &
                CLIENT_PIDS+=($!)
                sleep 0.5
            fi
            if command -v xcalc >/dev/null 2>&1; then
                echo " -> Spawning Window 2: xcalc (Stack pane top right)"
                DISPLAY="${XEPHYR_DISPLAY}" xcalc &
                CLIENT_PIDS+=($!)
                sleep 0.5
            fi
            if command -v xeyes >/dev/null 2>&1; then
                echo " -> Spawning Window 3: xeyes (Stack pane bottom right)"
                DISPLAY="${XEPHYR_DISPLAY}" xeyes &
                CLIENT_PIDS+=($!)
            fi
            ;;

        floating)
            echo "Spawning windows to demo Floating Window Layout..."
            if command -v xclock >/dev/null 2>&1; then
                echo " -> Spawning Window 1: xclock (Tiled background window)"
                DISPLAY="${XEPHYR_DISPLAY}" xclock &
                CLIENT_PIDS+=($!)
                sleep 0.5
            fi
            if command -v xcalc >/dev/null 2>&1; then
                echo " -> Spawning Window 2: xcalc (Press Mod4+Space to float)"
                DISPLAY="${XEPHYR_DISPLAY}" xcalc &
                CLIENT_PIDS+=($!)
            fi
            ;;

        all|*)
            echo "Spawning sample windows to test all implemented layouts..."
            if command -v xclock >/dev/null 2>&1; then
                echo " -> Spawning Window 1: xclock (Master pane: left 55%)"
                DISPLAY="${XEPHYR_DISPLAY}" xclock &
                CLIENT_PIDS+=($!)
                sleep 0.5
            fi
            if command -v xcalc >/dev/null 2>&1; then
                echo " -> Spawning Window 2: xcalc (Stack pane top right)"
                DISPLAY="${XEPHYR_DISPLAY}" xcalc &
                CLIENT_PIDS+=($!)
                sleep 0.5
            fi
            if command -v xeyes >/dev/null 2>&1; then
                echo " -> Spawning Window 3: xeyes (Stack pane bottom right)"
                DISPLAY="${XEPHYR_DISPLAY}" xeyes &
                CLIENT_PIDS+=($!)
            fi
            ;;
    esac

    echo ""
    echo "============================================================"
    echo " Xephyr Layout Demo active on display ${XEPHYR_DISPLAY}"
    echo " Demo Mode: ${LAYOUT_TYPE}"
    echo "============================================================"
    echo " Implemented Layouts under test:"
    echo "  1. Master/Stack Tiling Layout:"
    echo "     - 1 Window : Full usable screen area (single/monocle)"
    echo "     - 2 Windows: Master (left 55%) + Stack (right 45%)"
    echo "     - 3+ Windows: Master (left 55%) + Vertical Stack (right divided evenly)"
    echo "     - Dynamic Retiling: Closing/unmapping any window automatically retiles"
    echo "  2. Floating Layout (Phase 9):"
    echo "     - Press Mod4 + Space to toggle focused window between Tiled <-> Floating"
    echo "     - Floating windows stay centered (60% screen) and raised above tiled windows"
    echo "     - Move floating: Mod4 + Left Click drag"
    echo "     - Resize floating: Mod4 + Right Click drag"
    echo "  3. Workspaces (Phase 8):"
    echo "     - Workspaces 1-9 each preserve independent layout geometry"
    echo ""
    echo " Test Keybindings inside Xephyr:"
    echo "  Mod4 + Space            : Toggle Tiled <-> Floating"
    echo "  Mod4 + j / k            : Focus Down / Up (cycle panes)"
    echo "  Mod4 + h / l            : Focus Left (Master) / Right (Stack)"
    echo "  Mod4 + Shift + j / k    : Swap window Down / Up in stack"
    echo "  Mod4 + Shift + h / l    : Swap window Left / Right (Master <-> Stack)"
    echo "  Mod4 + 1 .. 9           : Switch workspace"
    echo "  Mod4 + Shift + 1 .. 9   : Move window to workspace"
    echo "  Mod4 + Shift + q        : Close window (graceful ICCCM WM_DELETE_WINDOW)"
    echo ""
    echo " Press Ctrl+C in this terminal to exit."
    echo "============================================================"

    # Wait for tilo process
    wait "${TILO_PID}" 2>/dev/null || true
else
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
fi
