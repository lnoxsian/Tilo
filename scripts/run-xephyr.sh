#!/usr/bin/env bash
# ==============================================================================
# tilo - Xephyr Development & Interactive Testing Harness
# ==============================================================================
# Features:
#  - Configurable screen resolutions: 1366x768, 1080p, 720p, 480p, or custom WxH
#  - Implemented tiling layout demos: single, master, stack, grid, tabbed, bsp,
#    rotate, floating, fullscreen, scratchpad, workspaces, mouse, theme, or all at once
#  - Dynamic layout switching (bsp, master, grid, tabbed, monocle, vertical, horizontal)
#  - Unified theme switching (Tokyo Night, Catppuccin, Nord, Gruvbox, Dracula, Solarized)
#  - Statusbar mouse interactions (workspace click/scroll, tab click/close/scroll)
#  - Live interactive CLI prompt to launch apps, switch layouts, and control the session
#  - Graceful WM shutdown and process cleanup
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Color codes
C_RESET=$'\033[0m'
C_BOLD=$'\033[1m'
C_RED=$'\033[31m'
C_GREEN=$'\033[32m'
C_YELLOW=$'\033[33m'
C_BLUE=$'\033[34m'
C_MAGENTA=$'\033[35m'
C_CYAN=$'\033[36m'
C_GRAY=$'\033[90m'

# Default configuration
SCREEN_PRESET="720p"
SCREEN_SIZE="1280x720"
BUILD_MODE="debug"
NO_BUILD=0
FORCE_BUILD=0
USE_DIST=0
DEMO_MODE=""
NON_INTERACTIVE=0
AUTOMATED_TEST=0
CUSTOM_CONFIG_FILE=""
ACTIVE_LAYOUT="bsp"
THEME_PRESET="tokyo-night"
THEME_NAME="Tokyo Night"
NUM_MONITORS=1

# Theme colors (defaults to Tokyo Night)
COLOR_BG="0x1a1b26"
COLOR_FG="0xc0caf5"
COLOR_ACTIVE_BG="0x7aa2f7"
COLOR_ACTIVE_FG="0x15161e"
COLOR_URGENT_BG="0xf7768e"
COLOR_INACTIVE_FG="0x555566"
COLOR_BORDER_FOCUSED="0x7aa2f7"
COLOR_BORDER_NORMAL="0x24283b"

XEPHYR_BIN="$(command -v Xephyr || true)"
SESSION_CONFIG_DIR=""
SESSION_CONFIG_FILE=""

# Resolves a resolution preset or custom geometry string to WxH
parse_resolution() {
    local input="$1"
    case "${input,,}" in
        1366|1366x768|wxga)
            SCREEN_PRESET="1366x768"
            SCREEN_SIZE="1366x768"
            ;;
        1080|1080p|fhd|1920|1920x1080)
            SCREEN_PRESET="1080p"
            SCREEN_SIZE="1920x1080"
            ;;
        720|720p|hd|1280|1280x720)
            SCREEN_PRESET="720p"
            SCREEN_SIZE="1280x720"
            ;;
        480|480p|sd|854|854x480)
            SCREEN_PRESET="480p"
            SCREEN_SIZE="854x480"
            ;;
        480p-4x3|640x480)
            SCREEN_PRESET="480p-4x3"
            SCREEN_SIZE="640x480"
            ;;
        [0-9]*x[0-9]*)
            SCREEN_PRESET="${input}"
            SCREEN_SIZE="${input}"
            ;;
        *)
            echo -e "${C_RED}Error: Unknown resolution '${input}'.${C_RESET}" >&2
            echo -e "Valid presets: ${C_CYAN}1366x768${C_RESET}, ${C_CYAN}1080p${C_RESET}, ${C_CYAN}720p${C_RESET}, ${C_CYAN}480p${C_RESET}, or custom ${C_CYAN}WIDTHxHEIGHT${C_RESET}." >&2
            exit 1
            ;;
    esac
}

# Applies theme presets (unified color palettes)
apply_theme() {
    local preset="$1"
    case "${preset,,}" in
        tokyo-night|tokyonight|tokyo|default)
            THEME_PRESET="tokyo-night"
            THEME_NAME="Tokyo Night"
            COLOR_BG="0x1a1b26"
            COLOR_FG="0xc0caf5"
            COLOR_ACTIVE_BG="0x7aa2f7"
            COLOR_ACTIVE_FG="0x15161e"
            COLOR_URGENT_BG="0xf7768e"
            COLOR_INACTIVE_FG="0x555566"
            COLOR_BORDER_FOCUSED="0x7aa2f7"
            COLOR_BORDER_NORMAL="0x24283b"
            ;;
        catppuccin|mocha)
            THEME_PRESET="catppuccin"
            THEME_NAME="Catppuccin Mocha"
            COLOR_BG="0x1e1e2e"
            COLOR_FG="0xcdd6f4"
            COLOR_ACTIVE_BG="0xcba6f7"
            COLOR_ACTIVE_FG="0x11111b"
            COLOR_URGENT_BG="0xf38ba8"
            COLOR_INACTIVE_FG="0x585b70"
            COLOR_BORDER_FOCUSED="0xcba6f7"
            COLOR_BORDER_NORMAL="0x313244"
            ;;
        nord)
            THEME_PRESET="nord"
            THEME_NAME="Nord"
            COLOR_BG="0x2e3440"
            COLOR_FG="0xd8dee9"
            COLOR_ACTIVE_BG="0x88c0d0"
            COLOR_ACTIVE_FG="0x2e3440"
            COLOR_URGENT_BG="0xbf616a"
            COLOR_INACTIVE_FG="0x4c566a"
            COLOR_BORDER_FOCUSED="0x88c0d0"
            COLOR_BORDER_NORMAL="0x3b4252"
            ;;
        gruvbox|gruvbox-dark)
            THEME_PRESET="gruvbox"
            THEME_NAME="Gruvbox Dark"
            COLOR_BG="0x282828"
            COLOR_FG="0xebdbb2"
            COLOR_ACTIVE_BG="0xd79921"
            COLOR_ACTIVE_FG="0x282828"
            COLOR_URGENT_BG="0xcc241d"
            COLOR_INACTIVE_FG="0x665c54"
            COLOR_BORDER_FOCUSED="0xd79921"
            COLOR_BORDER_NORMAL="0x3c3836"
            ;;
        dracula)
            THEME_PRESET="dracula"
            THEME_NAME="Dracula"
            COLOR_BG="0x282a36"
            COLOR_FG="0xf8f8f2"
            COLOR_ACTIVE_BG="0xbd93f9"
            COLOR_ACTIVE_FG="0x282a36"
            COLOR_URGENT_BG="0xff5555"
            COLOR_INACTIVE_FG="0x6272a4"
            COLOR_BORDER_FOCUSED="0xbd93f9"
            COLOR_BORDER_NORMAL="0x44475a"
            ;;
        solarized|solarized-dark)
            THEME_PRESET="solarized"
            THEME_NAME="Solarized Dark"
            COLOR_BG="0x002b36"
            COLOR_FG="0x839496"
            COLOR_ACTIVE_BG="0x268bd2"
            COLOR_ACTIVE_FG="0x002b36"
            COLOR_URGENT_BG="0xdc322f"
            COLOR_INACTIVE_FG="0x586e75"
            COLOR_BORDER_FOCUSED="0x268bd2"
            COLOR_BORDER_NORMAL="0x073642"
            ;;
        *)
            echo -e "${C_RED}Error: Unknown theme '${preset}'.${C_RESET}" >&2
            echo -e "Available presets: ${C_CYAN}tokyo-night, catppuccin, nord, gruvbox, dracula, solarized${C_RESET}" >&2
            return 1
            ;;
    esac
    return 0
}

# Writes or updates the session config file
write_session_config() {
    if [[ -z "${SESSION_CONFIG_FILE}" ]]; then
        return
    fi

    if [[ -n "${CUSTOM_CONFIG_FILE}" && -f "${CUSTOM_CONFIG_FILE}" ]]; then
        cp -f "${CUSTOM_CONFIG_FILE}" "${SESSION_CONFIG_FILE}"
        return
    fi

    cat << EOF > "${SESSION_CONFIG_FILE}"
# Generated session configuration for tilo Xephyr harness
mod_key Mod4
font monospace
terminal terminator
layout ${ACTIVE_LAYOUT}
border_width 2
inner_gap 8
outer_gap 8
master_ratio 0.55

# Statusbar & Mouse Interactions
workspace_mouse true
workspace_wheel_switch true
tab_mouse true
tab_wheel_switch true
tab_show_index true
tab_max_width 180

# Theme Configuration (${THEME_NAME})
bg ${COLOR_BG}
fg ${COLOR_FG}
active_bg ${COLOR_ACTIVE_BG}
active_fg ${COLOR_ACTIVE_FG}
urgent_bg ${COLOR_URGENT_BG}
inactive_fg ${COLOR_INACTIVE_FG}
border_focused ${COLOR_BORDER_FOCUSED}
border_normal ${COLOR_BORDER_NORMAL}
EOF
}

# Updates the layout and triggers a hot reload
set_layout() {
    local lt="$1"
    ACTIVE_LAYOUT="${lt}"
    write_session_config
    if [[ -n "${TILO_PID:-}" ]] && kill -0 "${TILO_PID}" 2>/dev/null; then
        kill -HUP "${TILO_PID}"
        echo -e "${C_GREEN}  [+] Switched layout to '${ACTIVE_LAYOUT}' and reloaded tilo (SIGHUP).${C_RESET}"
        sleep 0.2
    fi
}

# Updates the theme preset and triggers a hot reload
set_theme() {
    local preset="$1"
    if apply_theme "${preset}"; then
        write_session_config
        if [[ -n "${TILO_PID:-}" ]] && kill -0 "${TILO_PID}" 2>/dev/null; then
            kill -HUP "${TILO_PID}"
            echo -e "${C_GREEN}  [+] Applied theme '${THEME_NAME}' and reloaded tilo (SIGHUP).${C_RESET}"
            sleep 0.2
        fi
    fi
}

show_usage() {
    cat << EOF
${C_BOLD}tilo Xephyr Test & Demo Harness${C_RESET}

${C_YELLOW}Usage:${C_RESET}
  $0 [options]

${C_YELLOW}Resolution & Multi-Monitor Options:${C_RESET}
  -r, --res <preset|WxH>   Resolution preset (1366x768, 1080p, 720p, 480p) or custom WxH
  -m, --monitors [count]   Number of monitor windows to simulate in Xephyr (default: 2)
  --dual-monitor           Simulate 2 separate monitor windows (Xinerama multi-head)
  --multi-monitor          Alias for --dual-monitor
  --1366, --1366x768       Shortcut for 1366x768 (WXGA)
  --1080p, --1080          Shortcut for 1920x1080 (Full HD)
  --720p, --720            Shortcut for 1280x720 (HD, default)
  --480p, --480            Shortcut for 854x480 (SD 16:9)

${C_YELLOW}Layout & Demo Options:${C_RESET}
  -d, --demo [mode]        Launch layout demo inside Xephyr:
                           - ${C_CYAN}single${C_RESET}     : 1-window monocle tiling
                           - ${C_CYAN}master${C_RESET}     : 2-window Master/Stack split (55%/45%)
                           - ${C_CYAN}stack${C_RESET}      : 3-window Master + Vertical Stack
                           - ${C_CYAN}grid${C_RESET}       : Evenly divided rows & columns grid
                           - ${C_CYAN}tabbed${C_RESET}     : Tabbed window layer with statusbar tab pills
                           - ${C_CYAN}bsp${C_RESET}        : Binary space partitioning tree & split preselection
                           - ${C_CYAN}rotate${C_RESET}     : Master-stack orientation rotation (5 orientations)
                           - ${C_CYAN}floating${C_RESET}   : Tiled + Centered floating window layer
                           - ${C_CYAN}fullscreen${C_RESET} : Fullscreen mode toggle
                           - ${C_CYAN}scratchpad${C_RESET} : Dedicated dropdown/floating scratchpad
                           - ${C_CYAN}workspaces${C_RESET} : Multi-workspace client distribution
                           - ${C_CYAN}mouse${C_RESET}      : Statusbar mouse & tab clicks/wheel interaction
                           - ${C_CYAN}theme${C_RESET}      : Showcase theme configuration & presets
                           - ${C_CYAN}all${C_RESET}        : Comprehensive showcase of all features (default)
                           - ${C_CYAN}tour${C_RESET}       : Guided step-by-step interactive layout tour
  -l, --layout <type>      Initial layout: bsp (default), master, grid, tabbed, monocle, etc.
  --test-layout [mode]     Alias for --demo

${C_YELLOW}Theme & Configuration Options:${C_RESET}
  -t, --theme <preset>     Theme preset: tokyo-night, catppuccin, nord, gruvbox, dracula, solarized
  -c, --config <file>      Use custom configuration file for the Xephyr session

${C_YELLOW}General Options:${C_RESET}
  --dist                   Use pre-built release binary from dist/ (dist/tilo)
  --release                Build and run tilo in release mode (default: debug)
  --debug                  Run tilo in debug mode (default)
  --no-build               Skip checking or running cargo build (uses existing binary)
  --build                  Force rebuilding tilo before launch
  --non-interactive        Disable interactive CLI prompt (runs tilo until killed)
  --automated-tests        Run non-interactive cargo integration test suite
  -h, --help               Display this help message

${C_YELLOW}Live Interactive Prompt:${C_RESET}
  When started, this script provides a live command prompt to interactively
  launch applications into Xephyr, switch layout demos, adjust themes,
  or manage windows in real-time!
EOF
    exit 0
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        -r|--res|--screen|--resolution)
            if [[ $# -ge 2 ]] && [[ ! "$2" =~ ^- ]]; then
                parse_resolution "$2"
                shift 2
            else
                echo -e "${C_RED}Error: --res requires a resolution argument.${C_RESET}" >&2
                exit 1
            fi
            ;;
        -m|--monitors)
            if [[ $# -ge 2 ]] && [[ "$2" =~ ^[0-9]+$ ]]; then
                NUM_MONITORS="$2"
                shift 2
            else
                NUM_MONITORS=2
                shift 1
            fi
            ;;
        --dual-monitor|--dual-monitors|--multi-monitor|--multi-monitors)
            NUM_MONITORS=2
            shift 1
            ;;
        --1366|--1366x768)
            parse_resolution "1366x768"
            shift
            ;;
        --1080|--1080p)
            parse_resolution "1080p"
            shift
            ;;
        --720|--720p)
            parse_resolution "720p"
            shift
            ;;
        --480|--480p)
            parse_resolution "480p"
            shift
            ;;
        -d|--demo|--layout-demo|--test-layout|--test-layouts)
            if [[ $# -ge 2 ]] && [[ ! "$2" =~ ^- ]]; then
                DEMO_MODE="${2,,}"
                shift 2
            else
                DEMO_MODE="all"
                shift 1
            fi
            ;;
        -l|--layout)
            if [[ $# -ge 2 ]] && [[ ! "$2" =~ ^- ]]; then
                ACTIVE_LAYOUT="${2,,}"
                shift 2
            else
                echo -e "${C_RED}Error: --layout requires a layout name (bsp, master, grid, tabbed, monocle, etc.).${C_RESET}" >&2
                exit 1
            fi
            ;;
        -t|--theme)
            if [[ $# -ge 2 ]] && [[ ! "$2" =~ ^- ]]; then
                apply_theme "$2" || exit 1
                shift 2
            else
                echo -e "${C_RED}Error: --theme requires a preset name (tokyo-night, catppuccin, nord, gruvbox, dracula, solarized).${C_RESET}" >&2
                exit 1
            fi
            ;;
        -c|--config)
            if [[ $# -ge 2 ]] && [[ ! "$2" =~ ^- ]]; then
                CUSTOM_CONFIG_FILE="$2"
                shift 2
            else
                echo -e "${C_RED}Error: --config requires a file path argument.${C_RESET}" >&2
                exit 1
            fi
            ;;
        --dist)
            USE_DIST=1
            shift
            ;;
        --release)
            BUILD_MODE="release"
            shift
            ;;
        --debug)
            BUILD_MODE="debug"
            shift
            ;;
        --no-build|--fast)
            NO_BUILD=1
            shift
            ;;
        --build|--force-build)
            FORCE_BUILD=1
            shift
            ;;
        --non-interactive|--batch)
            NON_INTERACTIVE=1
            shift
            ;;
        --automated-tests|--test-cargo)
            AUTOMATED_TEST=1
            shift
            ;;
        -h|--help)
            show_usage
            ;;
        *)
            echo -e "${C_RED}Unknown option: $1${C_RESET}" >&2
            show_usage
            ;;
    esac
done

# Non-interactive automated cargo test mode
if [[ ${AUTOMATED_TEST} -eq 1 ]]; then
    echo -e "${C_BOLD}${C_BLUE}========================================================================${C_RESET}"
    echo -e "${C_BOLD} Running tilo layout, statusbar, and WM integration tests in Xephyr${C_RESET}"
    echo -e "${C_BOLD}${C_BLUE}========================================================================${C_RESET}"
    cd "${ROOT_DIR}"
    cargo test --test layout --test floating --test statusbar --test launcher --test tabbed --test scratchpad --test shutdown --test config --test mouse -- --nocapture
    echo ""
    echo -e "${C_GREEN}==> All integration tests passed successfully!${C_RESET}"
    exit 0
fi

# Ensure Xephyr is installed
if [[ -z "${XEPHYR_BIN}" ]]; then
    echo -e "${C_RED}Error: Xephyr is not installed on this system.${C_RESET}" >&2
    echo "Install it via your package manager (e.g. sudo apt install xserver-xephyr)." >&2
    exit 1
fi

# Find an available X11 display
XEPHYR_DISPLAY=""
for i in {1..99}; do
    if [[ ! -e "/tmp/.X11-unix/X${i}" && ! -e "/tmp/.X${i}-lock" ]]; then
        XEPHYR_DISPLAY=":${i}"
        break
    fi
done

if [[ -z "${XEPHYR_DISPLAY}" ]]; then
    echo -e "${C_RED}Error: Could not find an available X display for Xephyr.${C_RESET}" >&2
    exit 1
fi

# Resolve or build tilo binary
if [[ ${USE_DIST} -eq 1 ]]; then
    if [[ -x "${ROOT_DIR}/dist/tilo" ]]; then
        echo -e "${C_CYAN}--> Using exported release binary from dist/ (${ROOT_DIR}/dist/tilo)...${C_RESET}"
        TILO_BIN="${ROOT_DIR}/dist/tilo"
    else
        echo -e "${C_RED}Error: --dist requested, but '${ROOT_DIR}/dist/tilo' is not available or executable.${C_RESET}" >&2
        echo -e "Build and export it first with '${C_CYAN}make docker-build${C_RESET}' or '${C_CYAN}make docker-compose${C_RESET}'." >&2
        exit 1
    fi
else
    if [[ "${BUILD_MODE}" == "release" ]]; then
        TILO_BIN="${ROOT_DIR}/target/release/tilo"
    else
        TILO_BIN="${ROOT_DIR}/target/debug/tilo"
    fi

    # Determine if cargo build is needed:
    # 1. Always build if FORCE_BUILD=1
    # 2. Never build if NO_BUILD=1
    # 3. Otherwise build only if binary does not exist or source files are newer
    NEED_BUILD=0
    if [[ ${FORCE_BUILD} -eq 1 ]]; then
        NEED_BUILD=1
    elif [[ ${NO_BUILD} -eq 0 ]] && command -v cargo >/dev/null 2>&1; then
        if [[ ! -x "${TILO_BIN}" ]]; then
            NEED_BUILD=1
        elif find "${ROOT_DIR}/src" "${ROOT_DIR}/Cargo.toml" -type f -newer "${TILO_BIN}" 2>/dev/null | grep -q .; then
            NEED_BUILD=1
        fi
    fi

    if [[ ${NEED_BUILD} -eq 1 ]]; then
        echo -e "${C_CYAN}--> Building tilo (${BUILD_MODE} mode)...${C_RESET}"
        if [[ "${BUILD_MODE}" == "release" ]]; then
            (cd "${ROOT_DIR}" && cargo build --release) || true
        else
            (cd "${ROOT_DIR}" && cargo build) || true
        fi
    fi

    # Fallback to dist/ if target binary is not available or executable
    if [[ ! -x "${TILO_BIN}" ]]; then
        if [[ -x "${ROOT_DIR}/dist/tilo" ]]; then
            echo -e "${C_YELLOW}--> Target binary '${TILO_BIN}' not found. Using pre-built binary from dist/ (${ROOT_DIR}/dist/tilo)...${C_RESET}"
            TILO_BIN="${ROOT_DIR}/dist/tilo"
        elif [[ -x "${ROOT_DIR}/target/release/tilo" ]]; then
            echo -e "${C_YELLOW}--> Falling back to available release binary (${ROOT_DIR}/target/release/tilo)...${C_RESET}"
            TILO_BIN="${ROOT_DIR}/target/release/tilo"
        elif [[ -x "${ROOT_DIR}/target/debug/tilo" ]]; then
            echo -e "${C_YELLOW}--> Falling back to available debug binary (${ROOT_DIR}/target/debug/tilo)...${C_RESET}"
            TILO_BIN="${ROOT_DIR}/target/debug/tilo"
        else
            echo -e "${C_RED}Error: Neither '${TILO_BIN}' nor '${ROOT_DIR}/dist/tilo' is available or executable.${C_RESET}" >&2
            echo -e "Please build with '${C_CYAN}cargo build${C_RESET}', '${C_CYAN}make docker-build${C_RESET}', or run '${C_CYAN}make docker-compose${C_RESET}'." >&2
            exit 1
        fi
    fi
fi

# Create dedicated Xephyr session configuration directory
SESSION_CONFIG_DIR="$(mktemp -d -t tilo-xephyr-XXXXXX)"
mkdir -p "${SESSION_CONFIG_DIR}/tilo"
SESSION_CONFIG_FILE="${SESSION_CONFIG_DIR}/tilo/config"
write_session_config

XEPHYR_PID=""
TILO_PID=""
CLIENT_PIDS=()

cleanup() {
    echo ""
    echo -e "${C_YELLOW}Cleaning up Xephyr session (${XEPHYR_DISPLAY:-none})...${C_RESET}"
    for pid in "${CLIENT_PIDS[@]}"; do
        if kill -0 "${pid}" 2>/dev/null; then
            kill "${pid}" 2>/dev/null || true
        fi
    done
    if [[ -n "${TILO_PID:-}" ]] && kill -0 "${TILO_PID}" 2>/dev/null; then
        # Send SIGTERM to invoke graceful shutdown sequence in tilo
        kill -TERM "${TILO_PID}" 2>/dev/null || true
        for _ in {1..10}; do
            if ! kill -0 "${TILO_PID}" 2>/dev/null; then
                break
            fi
            sleep 0.05
        done
        kill -9 "${TILO_PID}" 2>/dev/null || true
    fi
    if [[ -n "${XEPHYR_PID:-}" ]] && kill -0 "${XEPHYR_PID}" 2>/dev/null; then
        kill "${XEPHYR_PID}" 2>/dev/null || true
    fi
    if [[ -n "${SESSION_CONFIG_DIR:-}" && -d "${SESSION_CONFIG_DIR}" ]]; then
        rm -rf "${SESSION_CONFIG_DIR}"
    fi
}
trap cleanup EXIT INT TERM

# Start Xephyr server
XEPHYR_SCREEN_ARGS=()
if [[ ${NUM_MONITORS} -gt 1 ]]; then
    IFS='x' read -r MON_W MON_H <<< "${SCREEN_SIZE}"
    XEPHYR_SCREEN_ARGS+=("+xinerama" "-screen" "${SCREEN_SIZE}")
    for (( i=1; i<NUM_MONITORS; i++ )); do
        OFFSET_X=$(( i * MON_W ))
        XEPHYR_SCREEN_ARGS+=("-origin" "${OFFSET_X},0" "-screen" "${SCREEN_SIZE}")
    done
    echo -e "${C_GREEN}--> Launching Xephyr multi-monitor (${NUM_MONITORS} separate windows, each ${SCREEN_SIZE}) on display ${XEPHYR_DISPLAY}...${C_RESET}"
else
    XEPHYR_SCREEN_ARGS+=("-screen" "${SCREEN_SIZE}")
    echo -e "${C_GREEN}--> Launching Xephyr on display ${XEPHYR_DISPLAY} (${SCREEN_SIZE}, preset: ${SCREEN_PRESET})...${C_RESET}"
fi

"${XEPHYR_BIN}" "${XEPHYR_DISPLAY}" \
    -ac \
    "${XEPHYR_SCREEN_ARGS[@]}" \
    -title "tilo [Xephyr ${XEPHYR_DISPLAY}]" \
    -resizeable \
    -br \
    -noreset &
XEPHYR_PID=$!

# Wait for Xephyr display socket to become ready (fast poll: 10ms intervals)
READY=0
for _ in {1..100}; do
    if [[ -S "/tmp/.X11-unix/X${XEPHYR_DISPLAY#:}" ]] || [[ -e "/tmp/.X11-unix/X${XEPHYR_DISPLAY#:}" ]]; then
        READY=1
        break
    fi
    sleep 0.01
done

if [[ ${READY} -eq 0 ]]; then
    echo -e "${C_YELLOW}Warning: Display socket wait timed out, continuing anyway...${C_RESET}"
fi

# Launch tilo window manager with custom session config directory
echo -e "${C_GREEN}--> Launching tilo window manager on ${XEPHYR_DISPLAY} (Layout: ${ACTIVE_LAYOUT}, Theme: ${THEME_NAME})...${C_RESET}"
XDG_CONFIG_HOME="${SESSION_CONFIG_DIR}" DISPLAY="${XEPHYR_DISPLAY}" "${TILO_BIN}" &
TILO_PID=$!

# Wait for tilo window manager to initialize (EWMH check or process readiness)
for _ in {1..50}; do
    if ! kill -0 "${TILO_PID}" 2>/dev/null; then
        break
    fi
    if command -v xprop >/dev/null 2>&1; then
        if xprop -display "${XEPHYR_DISPLAY}" -root _NET_SUPPORTING_WM_CHECK 2>/dev/null | grep -q "window id"; then
            break
        fi
    else
        break
    fi
    sleep 0.01
done

# Helper function to launch an app into the Xephyr session
launch_client() {
    local cmd=("$@")
    if command -v "${cmd[0]}" >/dev/null 2>&1; then
        DISPLAY="${XEPHYR_DISPLAY}" "${cmd[@]}" >/dev/null 2>&1 &
        local pid=$!
        CLIENT_PIDS+=("${pid}")
        echo -e "${C_GREEN}  [+] Started ${cmd[*]} (PID: ${pid})${C_RESET}"
        return 0
    else
        echo -e "${C_RED}  [-] Command not found: ${cmd[0]}${C_RESET}"
        return 1
    fi
}

# Helper function to spawn a terminal client
spawn_terminal() {
    local terms=(terminator foot alacritty kitty xterm urxvt st konsole gnome-terminal xfce4-terminal)
    for t in "${terms[@]}"; do
        if command -v "${t}" >/dev/null 2>&1; then
            launch_client "${t}"
            return 0
        fi
    done
    echo -e "${C_YELLOW}  [-] No standard terminal emulator found on PATH (${terms[*]}).${C_RESET}"
    return 1
}

# Clears all running client windows (keeps tilo and Xephyr running)
clean_clients() {
    echo -e "${C_YELLOW}Closing all demo client windows...${C_RESET}"
    for pid in "${CLIENT_PIDS[@]}"; do
        if kill -0 "${pid}" 2>/dev/null; then
            kill "${pid}" 2>/dev/null || true
        fi
    done
    CLIENT_PIDS=()
    sleep 0.2
}

# Layout demo runner
run_demo() {
    local mode="$1"
    clean_clients
    echo ""
    echo -e "${C_BOLD}${C_MAGENTA}=== Running Demo: ${mode} ===${C_RESET}"

    case "${mode}" in
        single|monocle|1)
            set_layout monocle
            echo -e "${C_CYAN}Demo: Single-Window Monocle Tiling${C_RESET}"
            echo "Window occupies the entire usable screen area below the statusbar."
            launch_client xclock || spawn_terminal || launch_client xcalc
            ;;

        master|split|two|2)
            set_layout master
            echo -e "${C_CYAN}Demo: 2-Window Master / Stack Split${C_RESET}"
            echo "Master pane: Left 55% | Stack pane: Right 45%"
            launch_client xclock
            sleep 0.25
            launch_client xcalc || spawn_terminal
            ;;

        stack|vertical|three|3)
            set_layout master
            echo -e "${C_CYAN}Demo: 3-Window Master + Vertical Stack${C_RESET}"
            echo "Master pane: Left 55% | Stack pane: Right divided vertically into 2 rows"
            launch_client xclock
            sleep 0.25
            launch_client xcalc
            sleep 0.25
            launch_client xeyes || spawn_terminal
            ;;

        grid|4)
            set_layout grid
            echo -e "${C_CYAN}Demo: Grid Layout${C_RESET}"
            echo "Windows are divided equally into a balanced grid of rows and columns."
            launch_client xclock
            sleep 0.25
            launch_client xcalc
            sleep 0.25
            launch_client xeyes
            sleep 0.25
            spawn_terminal || launch_client xcalc
            sleep 0.25
            echo ""
            echo -e "${C_GREEN}Active Setup:${C_RESET}"
            echo " - 4 windows tiled into an equal 2x2 grid."
            echo " - Layout indicator on statusbar shows '[G]'."
            echo -e "${C_YELLOW}Tip: Press Mod4 + Return to add windows, or Mod4 + q to close windows to see the grid dynamically rebalance.${C_RESET}"
            ;;

        tabbed|tabs|t)
            set_layout tabbed
            echo -e "${C_CYAN}Demo: Tabbed Layout${C_RESET}"
            echo "Windows share the full usable screen area; statusbar displays clickable tab pills."
            spawn_terminal || launch_client xclock
            sleep 0.25
            launch_client xclock
            sleep 0.25
            launch_client xcalc
            sleep 0.25
            echo ""
            echo -e "${C_GREEN}Active Setup:${C_RESET}"
            echo " - 3 windows opened; only the active window is visible and fills the screen."
            echo " - Layout indicator on statusbar shows '[T]'."
            echo " - Statusbar displays tab pills with numbers and titles: [1: ...] [2: ...] [3: ...]."
            echo -e "${C_YELLOW}Interactive Tab Controls:${C_RESET}"
            echo "   * Left-Click a tab pill on the statusbar to switch to it."
            echo "   * Middle-Click a tab pill to close that window."
            echo "   * Scroll mouse wheel over the tab region to cycle tabs."
            echo "   * Press Mod4 + Tab to focus next tab, Mod4 + Shift + Tab for previous tab."
            ;;

        bsp|tree|b)
            set_layout bsp
            echo -e "${C_CYAN}Demo: Binary Space Partitioning (BSP) Tree Layout${C_RESET}"
            echo "Windows recursively split active containers into a binary tree structure."
            spawn_terminal || launch_client xclock
            sleep 0.25
            launch_client xcalc
            sleep 0.25
            launch_client xclock
            sleep 0.25
            echo ""
            echo -e "${C_GREEN}Active Setup:${C_RESET}"
            echo " - 3 windows arranged in binary split hierarchy."
            echo " - Layout indicator on statusbar shows '[B]'."
            echo -e "${C_YELLOW}BSP Split Preselection:${C_RESET}"
            echo "   * Press Mod4 + v to preselect Vertical split for the next window."
            echo "   * Press Mod4 + b to preselect Horizontal split for the next window."
            echo -e "${C_YELLOW}BSP Tile Resizing:${C_RESET}"
            echo "   * Press Mod4 + Ctrl + Left / Right / Up / Down to dynamically adjust tile borders."
            ;;

        rotate|rot)
            set_layout master
            echo -e "${C_CYAN}Demo: Master-Stack Layout Orientations & Rotation${C_RESET}"
            launch_client xclock
            sleep 0.25
            launch_client xcalc
            sleep 0.25
            launch_client xeyes || spawn_terminal
            sleep 0.25
            echo ""
            echo -e "${C_GREEN}Active Setup:${C_RESET}"
            echo " - Master + Stack layout running."
            echo -e "${C_YELLOW}Tip: Press Mod4 + r inside Xephyr to rotate through 5 orientations:${C_RESET}"
            echo "   1. Master-Left   ('[]=') : Master on left, Stack on right (default)"
            echo "   2. Master-Right  ('=[]') : Master on right, Stack on left"
            echo "   3. Master-Top    ('TT=') : Master on top, Stack on bottom"
            echo "   4. Master-Bottom ('=TT') : Master on bottom, Stack on top"
            echo "   5. Center-Master ('|M|') : Master centered between left & right stacks"
            ;;

        floating|float)
            set_layout master
            echo -e "${C_CYAN}Demo: Floating Window Layering${C_RESET}"
            echo "1. Spawning tiled base window (xclock)"
            launch_client xclock
            sleep 0.25
            echo "2. Spawning floating client (xcalc)"
            launch_client xcalc
            sleep 0.25
            echo -e "${C_YELLOW}Tip: Press Mod4 + Shift + Space inside Xephyr to toggle focused window between Tiled <-> Floating!${C_RESET}"
            echo -e "${C_YELLOW}Tip: Drag with Mod4 + Left Click to move; Mod4 + Right Click to resize.${C_RESET}"
            ;;

        fullscreen|full)
            set_layout master
            echo -e "${C_CYAN}Demo: Fullscreen Mode${C_RESET}"
            echo "1. Spawning background client (xclock)"
            launch_client xclock
            sleep 0.25
            echo "2. Spawning target client (xterm / xcalc)"
            spawn_terminal || launch_client xcalc
            sleep 0.25
            echo -e "${C_YELLOW}Tip: Press Mod4 + f inside Xephyr to toggle Fullscreen mode for the focused window!${C_RESET}"
            ;;

        scratchpad|scratch)
            set_layout bsp
            echo -e "${C_CYAN}Demo: Dedicated Floating Scratchpad${C_RESET}"
            echo "1. Spawning base tiled client (xclock)"
            launch_client xclock
            sleep 0.25
            echo "2. Spawning client to designate as scratchpad (xcalc)"
            launch_client xcalc
            sleep 0.3
            echo ""
            echo -e "${C_YELLOW}Scratchpad Instructions:${C_RESET}"
            echo "   * With xcalc focused, press Mod4 + Shift + \` (grave/backtick) to assign it as the scratchpad."
            echo "   * Press Mod4 + \` (grave/backtick) anytime to toggle hiding and showing the scratchpad!"
            ;;

        mouse|statusbar-mouse)
            set_layout tabbed
            echo -e "${C_CYAN}Demo: Statusbar Mouse Interactions${C_RESET}"
            echo "1. Spawning client on Workspace 1"
            spawn_terminal || launch_client xclock
            sleep 0.25
            launch_client xcalc
            sleep 0.25
            echo ""
            show_mouse_guide
            ;;

        workspaces|ws)
            set_layout master
            echo -e "${C_CYAN}Demo: Workspaces 1..9${C_RESET}"
            echo "1. Spawning xclock on Workspace 1"
            launch_client xclock
            sleep 0.25
            echo -e "${C_YELLOW}Workspace Navigation Tips:${C_RESET}"
            echo " - Press Mod4 + 2 to switch to Workspace 2."
            echo " - Press Mod4 + Shift + 2 to move the focused window to Workspace 2."
            echo " - Or Left-Click on '2' in the statusbar to switch to Workspace 2!"
            echo " - Or Right-Click on '2' in the statusbar to send the focused window to Workspace 2!"
            echo " - Or Scroll mouse wheel over workspace numbers to cycle through workspaces!"
            ;;

        multi|monitors|multimonitor)
            set_layout bsp
            echo -e "${C_CYAN}Demo: Multi-Monitor Dual-Workspace Setup${C_RESET}"
            echo "1. Spawning master client (xclock) on Workspace 1"
            launch_client xclock
            sleep 0.25
            echo "2. Spawning secondary client (xcalc)"
            launch_client xcalc
            sleep 0.25
            echo ""
            echo -e "${C_GREEN}Multi-Monitor Controls in Tilo:${C_RESET}"
            echo "   * Move your mouse to either monitor window and press ${C_BOLD}Mod4 + d${C_RESET}:"
            echo "     The application launcher opens centered on whichever monitor your cursor is on!"
            echo "   * Press ${C_BOLD}Mod4 + period (.)${C_RESET} : Focus monitor to the right"
            echo "   * Press ${C_BOLD}Mod4 + comma (,)${C_RESET}  : Focus monitor to the left"
            echo "   * Press ${C_BOLD}Mod4 + Shift + period (.)${C_RESET} : Move focused window to right monitor"
            echo "   * Press ${C_BOLD}Mod4 + Shift + comma (,)${C_RESET}  : Move focused window to left monitor"
            echo "   * Press ${C_BOLD}Mod4 + Shift + 1..9${C_RESET} : Move focused window directly to target workspace"
            ;;

        theme|themes)
            echo -e "${C_CYAN}Available Theme Presets:${C_RESET}"
            echo "  - tokyo-night  : Tokyo Night (default)"
            echo "  - catppuccin   : Catppuccin Mocha"
            echo "  - nord         : Nord theme"
            echo "  - gruvbox      : Gruvbox Dark"
            echo "  - dracula      : Dracula theme"
            echo "  - solarized    : Solarized Dark"
            echo ""
            echo "Type 'theme <preset>' in the interactive prompt to switch themes in real-time!"
            ;;

        all|all-at-once|full-showcase)
            set_layout bsp
            echo -e "${C_CYAN}Demo: Comprehensive Showcase (All Modes)${C_RESET}"
            echo "1. Spawning master client (xclock)"
            launch_client xclock
            sleep 0.25
            echo "2. Spawning secondary client (xcalc)"
            launch_client xcalc
            sleep 0.25
            echo "3. Spawning tertiary client (xeyes)"
            launch_client xeyes || spawn_terminal
            sleep 0.25
            echo ""
            echo -e "${C_GREEN}Active Setup:${C_RESET}"
            echo " - Statusbar at top with active workspace pill, occupied tags, layout indicator, metrics, and clock"
            echo " - Windows tiled in binary space partitioning layout ('[B]')"
            echo " - Press Mod4 + Space to cycle layouts (BSP -> Master -> Grid -> Tabbed -> Monocle)"
            echo " - Press Mod4 + Shift + Space to toggle floating layer"
            echo " - Press Mod4 + d to open the Application Launcher"
            echo " - Press Mod4 + Shift + q to open the Power/Exit menu"
            ;;

        tour|cycle)
            echo -e "${C_CYAN}=== Starting Guided Layout Tour ===${C_RESET}"
            echo -e "We will cycle through every implemented layout and feature step-by-step."
            echo ""
            read -rp "Press Enter to start Step 1: Single Window Monocle... " _
            run_demo single
            read -rp "Press Enter to proceed to Step 2: 2-Window Master/Stack Split... " _
            run_demo master
            read -rp "Press Enter to proceed to Step 3: 3-Window Vertical Stack... " _
            run_demo stack
            read -rp "Press Enter to proceed to Step 4: Grid Layout (Rows & Columns)... " _
            run_demo grid
            read -rp "Press Enter to proceed to Step 5: Tabbed Layout & Statusbar Tabs... " _
            run_demo tabbed
            read -rp "Press Enter to proceed to Step 6: Binary Space Partitioning (BSP)... " _
            run_demo bsp
            read -rp "Press Enter to proceed to Step 7: Master-Stack Rotation Orientations... " _
            run_demo rotate
            read -rp "Press Enter to proceed to Step 8: Floating Window Layer... " _
            run_demo floating
            read -rp "Press Enter to proceed to Step 9: Fullscreen Mode... " _
            run_demo fullscreen
            read -rp "Press Enter to proceed to Step 10: Dedicated Floating Scratchpad... " _
            run_demo scratchpad
            read -rp "Press Enter to proceed to Step 11: Statusbar Mouse Interactions... " _
            run_demo mouse
            read -rp "Press Enter to proceed to Step 12: Workspaces 1..9... " _
            run_demo workspaces
            read -rp "Press Enter to finish Tour with Full Showcase... " _
            run_demo all
            echo -e "${C_GREEN}Tour complete!${C_RESET}"
            ;;

        *)
            echo -e "${C_RED}Unknown demo mode: '${mode}'${C_RESET}"
            echo "Available modes: single, master, stack, grid, tabbed, bsp, rotate, floating, fullscreen, scratchpad, mouse, workspaces, theme, all, tour"
            ;;
    esac
}

show_keybindings() {
    cat << EOF
${C_BOLD}${C_YELLOW}tilo Default Keybindings inside Xephyr:${C_RESET}
  ${C_BOLD}Applications & WM Control:${C_RESET}
    ${C_CYAN}Mod4 + Return${C_RESET}           Spawn terminal (terminator / foot / alacritty / kitty / xterm)
    ${C_CYAN}Mod4 + d${C_RESET}                Toggle Application Launcher (search & spawn applications)
    ${C_CYAN}Mod4 + q${C_RESET}                Close focused window (graceful ICCCM delete)
    ${C_CYAN}Mod4 + Shift + q${C_RESET}        Open Exit/Power Menu in Launcher (Logout, Reboot, Poweroff)
    ${C_CYAN}Mod4 + Shift + r${C_RESET}        Hot-reload tilo configuration file

  ${C_BOLD}Layouts & Orientations:${C_RESET}
    ${C_CYAN}Mod4 + Space${C_RESET}            Cycle window layouts (BSP -> Master -> Grid -> Tabbed -> Monocle)
    ${C_CYAN}Mod4 + r${C_RESET}                Rotate master layout orientation ([]=, =[], TT=, =TT, |M|)
    ${C_CYAN}Mod4 + Shift + Space${C_RESET}    Toggle focused window between Tiled <-> Floating mode
    ${C_CYAN}Mod4 + f${C_RESET}                Toggle Fullscreen mode for focused window

  ${C_BOLD}Tab Navigation (in Tabbed Layout):${C_RESET}
    ${C_CYAN}Mod4 + Tab${C_RESET}              Focus next tab
    ${C_CYAN}Mod4 + Shift + Tab${C_RESET}      Focus previous tab

  ${C_BOLD}Dedicated Floating Scratchpad:${C_RESET}
    ${C_CYAN}Mod4 + \`${C_RESET} (backtick)       Toggle scratchpad window visibility (hide / show)
    ${C_CYAN}Mod4 + Shift + \`${C_RESET} (tilde) Assign currently focused window as the dedicated scratchpad

  ${C_BOLD}BSP Layout Controls:${C_RESET}
    ${C_CYAN}Mod4 + v${C_RESET}                Preselect Vertical split for next window
    ${C_CYAN}Mod4 + b${C_RESET}                Preselect Horizontal split for next window
    ${C_CYAN}Mod4 + Ctrl + Arrows${C_RESET}    Resize active tile boundary (Left / Right / Up / Down)

  ${C_BOLD}Spatial Focus & Window Movement:${C_RESET}
    ${C_CYAN}Mod4 + j / k or Down/Up${C_RESET} Focus Next / Previous window (Down / Up in stack)
    ${C_CYAN}Mod4 + h / l or Left/Rt${C_RESET} Focus Left / Right
    ${C_CYAN}Mod4 + Shift + j/k/h/l${C_RESET}  Swap / Move focused window in direction

  ${C_BOLD}Workspaces (1-9):${C_RESET}
    ${C_CYAN}Mod4 + 1 .. 9${C_RESET}           Switch to Workspace 1..9
    ${C_CYAN}Mod4 + Shift + 1 .. 9${C_RESET}   Move focused window to Workspace 1..9

  ${C_BOLD}Multi-Monitor Focus:${C_RESET}
    ${C_CYAN}Mod4 + , (comma)${C_RESET}        Focus visually Left Monitor
    ${C_CYAN}Mod4 + . (period)${C_RESET}       Focus visually Right Monitor

  ${C_BOLD}Mouse Controls:${C_RESET}
    ${C_CYAN}Mod4 + Left Drag${C_RESET}        Move floating window
    ${C_CYAN}Mod4 + Right Drag${C_RESET}       Resize floating window

${C_YELLOW}Tips for Xephyr Testing:${C_RESET}
  - If your desktop intercepts Super/Mod4, press ${C_CYAN}Ctrl + Shift${C_RESET} inside Xephyr
    to capture/release the keyboard and mouse from your host OS.
  - Or type commands directly in the interactive prompt below!
EOF
}

show_mouse_guide() {
    cat << EOF
${C_BOLD}${C_YELLOW}tilo Statusbar & Window Mouse Interactions:${C_RESET}
  ${C_BOLD}Statusbar Workspaces:${C_RESET}
    ${C_CYAN}Left-Click [1..9]${C_RESET}       Switch directly to the clicked workspace
    ${C_CYAN}Shift + Left-Click${C_RESET}      Move the focused window to the clicked workspace
    ${C_CYAN}Right-Click [1..9]${C_RESET}      Move the focused window to the clicked workspace
    ${C_CYAN}Scroll Wheel Up${C_RESET}         Switch to previous workspace (wraps around 1 <-> 9)
    ${C_CYAN}Scroll Wheel Down${C_RESET}       Switch to next workspace (wraps around 9 <-> 1)

  ${C_BOLD}Statusbar Tab Pills (in Tabbed Layout):${C_RESET}
    ${C_CYAN}Left-Click Tab${C_RESET}          Focus that tabbed window
    ${C_CYAN}Middle-Click Tab${C_RESET}        Close that tabbed window (graceful ICCCM delete)
    ${C_CYAN}Scroll Wheel Up/Down${C_RESET}    Cycle through tabs

  ${C_BOLD}Floating Window Controls:${C_RESET}
    ${C_CYAN}Mod4 + Left Drag${C_RESET}        Move floating window interactively
    ${C_CYAN}Mod4 + Right Drag${C_RESET}       Resize floating window interactively
EOF
}

show_session_info() {
    echo -e "${C_BOLD}Xephyr Session Info:${C_RESET}"
    echo -e "  Display    : ${C_GREEN}${XEPHYR_DISPLAY}${C_RESET}"
    echo -e "  Resolution : ${C_GREEN}${SCREEN_SIZE}${C_RESET} (Preset: ${SCREEN_PRESET})"
    if [[ ${NUM_MONITORS} -gt 1 ]]; then
        echo -e "  Monitors   : ${C_GREEN}${NUM_MONITORS} separate windows${C_RESET} (Xinerama multi-head)"
    else
        echo -e "  Monitors   : 1 window"
    fi
    echo -e "  Layout     : ${C_CYAN}${ACTIVE_LAYOUT}${C_RESET}"
    echo -e "  Theme      : ${C_CYAN}${THEME_NAME}${C_RESET} (Preset: ${THEME_PRESET})"
    echo -e "  Config     : ${C_GRAY}${SESSION_CONFIG_FILE}${C_RESET}"
    echo -e "  Xephyr PID : ${XEPHYR_PID}"
    echo -e "  tilo PID   : ${TILO_PID}"
    echo -e "  Client PIDs: ${#CLIENT_PIDS[@]} active (${CLIENT_PIDS[*]:-none})"
}

show_help() {
    echo -e "${C_BOLD}Interactive Prompt Commands:${C_RESET}"
    echo -e "  ${C_CYAN}launcher / app${C_RESET}       Toggle application launcher (SIGUSR1)"
    echo -e "  ${C_CYAN}layout <type>${C_RESET}        Switch layout (bsp, master, grid, tabbed, monocle, etc.)"
    echo -e "  ${C_CYAN}theme [preset]${C_RESET}       Switch theme (tokyo-night, catppuccin, nord, gruvbox, dracula, solarized)"
    echo -e "  ${C_CYAN}demo <mode>${C_RESET}          Run demo (single, master, stack, grid, tabbed, bsp, rotate, multi, scratchpad, mouse, workspaces, all, tour)"
    echo -e "  ${C_CYAN}multi / monitors${C_RESET}     Show multi-monitor controls and test demo"
    echo -e "  ${C_CYAN}grid / tabbed / bsp${C_RESET}  Direct shortcuts to new layout demos"
    echo -e "  ${C_CYAN}rotate / scratchpad${C_RESET}  Direct shortcuts to master rotation & scratchpad demos"
    echo -e "  ${C_CYAN}mouse${C_RESET}                Show statusbar mouse interaction guide"
    echo -e "  ${C_CYAN}term / xterm${C_RESET}         Spawn terminal in Xephyr"
    echo -e "  ${C_CYAN}clock / calc / eyes${C_RESET}  Spawn sample app in Xephyr"
    echo -e "  ${C_CYAN}launch <cmd>${C_RESET}         Run arbitrary command in Xephyr (e.g. launch htop)"
    echo -e "  ${C_CYAN}clean / clear${C_RESET}        Close all demo client windows"
    echo -e "  ${C_CYAN}reload${C_RESET}               Hot-reload tilo configuration (SIGHUP)"
    echo -e "  ${C_CYAN}test [suite]${C_RESET}         Run integration tests directly"
    echo -e "  ${C_CYAN}keys${C_RESET}                 Show keybindings reference"
    echo -e "  ${C_CYAN}info / status${C_RESET}        Show active session status"
    echo -e "  ${C_CYAN}quit / exit / q${C_RESET}      Gracefully shut down session and exit"
}

# If an initial demo mode was specified on CLI, run it now
if [[ -n "${DEMO_MODE}" ]]; then
    run_demo "${DEMO_MODE}"
fi

# Non-interactive mode just waits for tilo
if [[ ${NON_INTERACTIVE} -eq 1 ]]; then
    echo -e "${C_GREEN}Xephyr session running in non-interactive mode on ${XEPHYR_DISPLAY}.${C_RESET}"
    echo -e "Press Ctrl+C to stop."
    wait "${TILO_PID}" 2>/dev/null || true
    exit 0
fi

# Print banner and help for interactive live prompt
echo ""
echo -e "${C_BOLD}${C_BLUE}========================================================================${C_RESET}"
echo -e "${C_BOLD}${C_GREEN} Xephyr Session Ready on ${XEPHYR_DISPLAY} (${SCREEN_SIZE}) ${C_RESET}"
echo -e "${C_BOLD}${C_BLUE}========================================================================${C_RESET}"
show_session_info
echo ""
show_help
echo -e "${C_BLUE}========================================================================${C_RESET}"
echo ""

# Interactive CLI loop
while true; do
    # Check if Xephyr or tilo died unexpectedly
    if ! kill -0 "${XEPHYR_PID}" 2>/dev/null; then
        echo -e "${C_RED}Xephyr server closed. Exiting...${C_RESET}"
        break
    fi
    if ! kill -0 "${TILO_PID}" 2>/dev/null; then
        echo -e "${C_YELLOW}tilo process terminated. Exiting...${C_RESET}"
        break
    fi

    PROMPT_LABEL="${C_BOLD}${C_CYAN}tilo-xephyr [${SCREEN_PRESET} ${XEPHYR_DISPLAY} ${ACTIVE_LAYOUT}]>${C_RESET} "
    if ! read -r -e -p "$(echo -e "${PROMPT_LABEL}")" user_input; then
        # EOF received (Ctrl+D)
        echo ""
        break
    fi

    # Trim leading/trailing whitespace
    user_input="$(echo "${user_input}" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"

    if [[ -z "${user_input}" ]]; then
        continue
    fi

    # Parse command and arguments
    read -r cmd args <<< "${user_input}"

    case "${cmd}" in
        q|quit|exit|shutdown)
            echo -e "${C_GREEN}Shutting down Xephyr session...${C_RESET}"
            break
            ;;

        help|\?)
            show_help
            ;;

        launcher|app|dmenu|rofi)
            if [[ -n "${TILO_PID}" ]] && kill -0 "${TILO_PID}" 2>/dev/null; then
                kill -USR1 "${TILO_PID}"
                echo -e "${C_GREEN}  [+] Toggled application launcher (SIGUSR1 -> tilo PID ${TILO_PID})${C_RESET}"
            else
                echo -e "${C_RED}  [-] tilo is not running.${C_RESET}"
            fi
            ;;

        exitmenu|power|powermenu)
            echo -e "${C_CYAN}  Tip: Press Mod4 + Shift + q inside Xephyr to open the power/exit menu (Logout/Reboot/Poweroff).${C_RESET}"
            echo -e "${C_CYAN}  Or type 'quit' here to gracefully shut down the Xephyr session.${C_RESET}"
            ;;

        reload)
            if [[ -n "${TILO_PID}" ]] && kill -0 "${TILO_PID}" 2>/dev/null; then
                kill -HUP "${TILO_PID}"
                echo -e "${C_GREEN}  [+] Sent SIGHUP to tilo PID ${TILO_PID} to reload configuration.${C_RESET}"
            else
                echo -e "${C_RED}  [-] tilo is not running.${C_RESET}"
            fi
            ;;

        layout)
            if [[ -n "${args}" ]]; then
                set_layout "${args}"
            else
                echo -e "Current layout: ${C_GREEN}${ACTIVE_LAYOUT}${C_RESET}"
                echo -e "Available layouts: ${C_CYAN}bsp, master, grid, tabbed, monocle, vertical, horizontal, center-master${C_RESET}"
            fi
            ;;

        theme)
            if [[ -n "${args}" ]]; then
                set_theme "${args}"
            else
                echo -e "Current theme: ${C_GREEN}${THEME_NAME}${C_RESET} (${THEME_PRESET})"
                echo -e "Available presets: ${C_CYAN}tokyo-night, catppuccin, nord, gruvbox, dracula, solarized${C_RESET}"
                echo "Usage: theme <preset>"
            fi
            ;;

        test|tests)
            echo -e "${C_CYAN}--> Running integration tests...${C_RESET}"
            if [[ -n "${args}" ]]; then
                (cd "${ROOT_DIR}" && cargo test --test "${args}" -- --nocapture) || true
            else
                (cd "${ROOT_DIR}" && cargo test --test layout --test floating --test statusbar --test launcher --test tabbed --test scratchpad --test shutdown --test config --test mouse -- --nocapture) || true
            fi
            ;;

        demo)
            if [[ -n "${args}" ]]; then
                run_demo "${args}"
            else
                echo -e "Please specify demo type: ${C_CYAN}single, master, stack, grid, tabbed, bsp, rotate, floating, fullscreen, scratchpad, mouse, workspaces, theme, all, tour${C_RESET}"
            fi
            ;;

        grid)
            run_demo grid
            ;;
        tabbed|tabs)
            run_demo tabbed
            ;;
        bsp)
            run_demo bsp
            ;;
        rotate|rot)
            run_demo rotate
            ;;
        scratchpad|scratch)
            run_demo scratchpad
            ;;
        mouse|statusbar-mouse)
            run_demo mouse
            ;;
        single)
            run_demo single
            ;;
        master)
            run_demo master
            ;;
        stack)
            run_demo stack
            ;;
        floating|float)
            run_demo floating
            ;;
        fullscreen|full)
            run_demo fullscreen
            ;;
        workspaces|ws)
            run_demo workspaces
            ;;
        multi|monitors|multimonitor)
            run_demo multi
            ;;
        all)
            run_demo all
            ;;
        tour|cycle)
            run_demo tour
            ;;

        term|terminal|xterm)
            spawn_terminal
            ;;

        clock|xclock)
            launch_client xclock
            ;;

        calc|xcalc)
            launch_client xcalc
            ;;

        eyes|xeyes)
            launch_client xeyes
            ;;

        clean|clear)
            clean_clients
            ;;

        keys|keybindings)
            show_keybindings
            ;;

        mouseguide)
            show_mouse_guide
            ;;

        info|status)
            show_session_info
            ;;

        launch|spawn|run)
            if [[ -n "${args}" ]]; then
                # shellcheck disable=SC2086
                launch_client ${args}
            else
                echo -e "${C_RED}Usage: launch <command [args...]>${C_RESET}"
            fi
            ;;

        *)
            # Try to run directly if the command exists on system
            if command -v "${cmd}" >/dev/null 2>&1; then
                # shellcheck disable=SC2086
                launch_client ${user_input}
            else
                echo -e "${C_RED}Unknown command: '${user_input}'. Type 'help' for available commands.${C_RESET}"
            fi
            ;;
    esac
done
