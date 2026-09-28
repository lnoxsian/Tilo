#!/usr/bin/env bash
# ==============================================================================
# tilo - Xephyr Development & Interactive Testing Harness
# ==============================================================================
# Features:
#  - Configurable screen resolutions: 1366x768, 1080p, 720p, 480p, or custom WxH
#  - Implemented tiling layout demos: single, master, stack, floating, fullscreen,
#    workspaces, or all at once
#  - Live interactive CLI prompt to launch apps, switch layouts, and control the session
#  - Clean signal handling and process cleanup
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
DEMO_MODE=""
NON_INTERACTIVE=0
AUTOMATED_TEST=0
XEPHYR_BIN="$(command -v Xephyr || true)"

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

show_usage() {
    cat << EOF
${C_BOLD}tilo Xephyr Test & Demo Harness${C_RESET}

${C_YELLOW}Usage:${C_RESET}
  $0 [options]

${C_YELLOW}Resolution Options:${C_RESET}
  -r, --res <preset|WxH>   Resolution preset (1366x768, 1080p, 720p, 480p) or custom WxH
  --1366, --1366x768       Shortcut for 1366x768 (WXGA)
  --1080p, --1080          Shortcut for 1920x1080 (Full HD)
  --720p, --720            Shortcut for 1280x720 (HD, default)
  --480p, --480            Shortcut for 854x480 (SD 16:9)

${C_YELLOW}Layout Demo Options:${C_RESET}
  -d, --demo [mode]        Launch layout demo inside Xephyr:
                           - ${C_CYAN}single${C_RESET}     : 1-window monocle tiling
                           - ${C_CYAN}master${C_RESET}     : 2-window Master/Stack split (55%/45%)
                           - ${C_CYAN}stack${C_RESET}      : 3-window Master + Vertical Stack
                           - ${C_CYAN}floating${C_RESET}   : Tiled + Centered floating window layer
                           - ${C_CYAN}fullscreen${C_RESET} : Fullscreen mode toggle
                           - ${C_CYAN}workspaces${C_RESET} : Multi-workspace client distribution
                           - ${C_CYAN}all${C_RESET}        : Comprehensive showcase of all features (default)
                           - ${C_CYAN}tour${C_RESET}       : Step-by-step interactive guided layout tour
  --test-layout [mode]     Alias for --demo

${C_YELLOW}General Options:${C_RESET}
  --release                Build and run tilo in release mode (default: debug)
  --non-interactive        Disable interactive CLI prompt (runs tilo until killed)
  --automated-tests        Run non-interactive cargo test suite
  -h, --help               Display this help message

${C_YELLOW}Live Interactive Prompt:${C_RESET}
  When started, this script provides a live command prompt to interactively
  launch applications into Xephyr, switch layout demos, or manage windows!
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
        -d|--demo|--layout-demo|--test-layout|--layout|--test-layouts)
            if [[ $# -ge 2 ]] && [[ ! "$2" =~ ^- ]]; then
                DEMO_MODE="${2,,}"
                shift 2
            else
                DEMO_MODE="all"
                shift 1
            fi
            ;;
        --release)
            BUILD_MODE="release"
            shift
            ;;
        --debug)
            BUILD_MODE="debug"
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
    echo -e "${C_BOLD}${C_BLUE}============================================================${C_RESET}"
    echo -e "${C_BOLD} Running tilo layout and rendering integration tests in Xephyr${C_RESET}"
    echo -e "${C_BOLD}${C_BLUE}============================================================${C_RESET}"
    cd "${ROOT_DIR}"
    cargo test --test layout --test floating --test statusbar --test launcher -- --nocapture
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

# Build tilo before launching Xephyr
echo -e "${C_CYAN}--> Building tilo (${BUILD_MODE} mode)...${C_RESET}"
if [[ "${BUILD_MODE}" == "release" ]]; then
    (cd "${ROOT_DIR}" && cargo build --release)
    TILO_BIN="${ROOT_DIR}/target/release/tilo"
else
    (cd "${ROOT_DIR}" && cargo build)
    TILO_BIN="${ROOT_DIR}/target/debug/tilo"
fi

XEPHYR_PID=""
TILO_PID=""
CLIENT_PIDS=()

cleanup() {
    echo ""
    echo -e "${C_YELLOW}Cleaning up Xephyr session (${XEPHYR_DISPLAY})...${C_RESET}"
    for pid in "${CLIENT_PIDS[@]}"; do
        if kill -0 "${pid}" 2>/dev/null; then
            kill "${pid}" 2>/dev/null || true
        fi
    done
    if [[ -n "${TILO_PID}" ]] && kill -0 "${TILO_PID}" 2>/dev/null; then
        kill "${TILO_PID}" 2>/dev/null || true
    fi
    if [[ -n "${XEPHYR_PID}" ]] && kill -0 "${XEPHYR_PID}" 2>/dev/null; then
        kill "${XEPHYR_PID}" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

# Start Xephyr server
echo -e "${C_GREEN}--> Launching Xephyr on display ${XEPHYR_DISPLAY} (${SCREEN_SIZE}, preset: ${SCREEN_PRESET})...${C_RESET}"
"${XEPHYR_BIN}" "${XEPHYR_DISPLAY}" \
    -ac \
    -screen "${SCREEN_SIZE}" \
    -title "tilo [Xephyr ${XEPHYR_DISPLAY}]" \
    -resizeable \
    -br \
    -noreset &
XEPHYR_PID=$!

# Wait for Xephyr display socket to become ready
READY=0
for _ in {1..60}; do
    if [[ -S "/tmp/.X11-unix/X${XEPHYR_DISPLAY#:}" ]] || [[ -e "/tmp/.X11-unix/X${XEPHYR_DISPLAY#:}" ]]; then
        READY=1
        break
    fi
    sleep 0.05
done

if [[ ${READY} -eq 0 ]]; then
    echo -e "${C_YELLOW}Warning: Display socket wait timed out, continuing anyway...${C_RESET}"
fi

# Launch tilo window manager
echo -e "${C_GREEN}--> Launching tilo window manager on ${XEPHYR_DISPLAY}...${C_RESET}"
DISPLAY="${XEPHYR_DISPLAY}" "${TILO_BIN}" &
TILO_PID=$!
sleep 0.4

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
    echo -e "${C_BOLD}${C_MAGENTA}=== Running Layout Demo: ${mode} ===${C_RESET}"

    case "${mode}" in
        single|monocle|1)
            echo -e "${C_CYAN}Demo: Single-Window Monocle Tiling${C_RESET}"
            echo "Window occupies the entire usable screen area below the statusbar."
            launch_client xclock || launch_client xterm || launch_client xcalc
            ;;

        master|split|two|2)
            echo -e "${C_CYAN}Demo: 2-Window Master / Stack Split${C_RESET}"
            echo "Master pane: Left 55% | Stack pane: Right 45%"
            launch_client xclock
            sleep 0.3
            launch_client xcalc || launch_client xterm
            ;;

        stack|vertical|three|3)
            echo -e "${C_CYAN}Demo: 3-Window Master + Vertical Stack${C_RESET}"
            echo "Master pane: Left 55% | Stack pane: Right divided vertically into 2 rows"
            launch_client xclock
            sleep 0.3
            launch_client xcalc
            sleep 0.3
            launch_client xeyes || launch_client xterm
            ;;

        floating|float)
            echo -e "${C_CYAN}Demo: Floating Window Layering (Phase 9)${C_RESET}"
            echo "1. Spawning tiled base window (xclock)"
            launch_client xclock
            sleep 0.3
            echo "2. Spawning floating client (xcalc / xmessage)"
            launch_client xcalc
            sleep 0.3
            echo -e "${C_YELLOW}Tip: Press Mod4 + Space inside Xephyr to toggle focused window between Tiled <-> Floating!${C_RESET}"
            echo -e "${C_YELLOW}Tip: Drag with Mod4 + Left Click to move; Mod4 + Right Click to resize.${C_RESET}"
            ;;

        fullscreen|full)
            echo -e "${C_CYAN}Demo: Fullscreen Mode (Phase 13)${C_RESET}"
            echo "1. Spawning background client (xclock)"
            launch_client xclock
            sleep 0.3
            echo "2. Spawning target client (xterm / xcalc)"
            launch_client xterm || launch_client xcalc
            sleep 0.3
            echo -e "${C_YELLOW}Tip: Press Mod4 + f inside Xephyr to toggle Fullscreen mode for the focused window!${C_RESET}"
            ;;

        workspaces|ws)
            echo -e "${C_CYAN}Demo: Workspaces 1..9 (Phase 8)${C_RESET}"
            echo "1. Spawning xclock on Workspace 1"
            launch_client xclock
            sleep 0.3
            echo -e "${C_YELLOW}Tip: Press Mod4 + 2 inside Xephyr to switch to Workspace 2.${C_RESET}"
            echo -e "${C_YELLOW}Tip: Press Mod4 + Shift + 2 to move the focused window to Workspace 2.${C_RESET}"
            echo "Notice how the statusbar highlights active workspace pill and marks occupied workspaces with '*'!"
            ;;

        all|all-at-once|full-showcase)
            echo -e "${C_CYAN}Demo: Comprehensive Showcase (All Modes)${C_RESET}"
            echo "1. Master pane (xclock)"
            launch_client xclock
            sleep 0.3
            echo "2. Stack pane 1 (xcalc)"
            launch_client xcalc
            sleep 0.3
            echo "3. Stack pane 2 (xeyes)"
            launch_client xeyes || launch_client xterm
            sleep 0.3
            echo ""
            echo -e "${C_GREEN}Active Setup:${C_RESET}"
            echo " - Statusbar at top with active workspace pill, occupied tags, layout indicator, and metrics"
            echo " - 3 Tiled windows arranged in Master/Stack split (55%/45%)"
            echo " - Press Mod4 + Space to float any window, Mod4 + f to toggle fullscreen"
            ;;

        tour|cycle)
            echo -e "${C_CYAN}=== Starting Guided Layout Tour ===${C_RESET}"
            echo -e "We will cycle through each implemented layout step-by-step."
            echo ""
            read -rp "Press Enter to start Step 1: Single Window Monocle... " _
            run_demo single
            read -rp "Press Enter to proceed to Step 2: 2-Window Master/Stack Split... " _
            run_demo master
            read -rp "Press Enter to proceed to Step 3: 3-Window Vertical Stack... " _
            run_demo stack
            read -rp "Press Enter to proceed to Step 4: Floating Window Layer... " _
            run_demo floating
            read -rp "Press Enter to proceed to Step 5: Fullscreen Mode... " _
            run_demo fullscreen
            read -rp "Press Enter to finish Tour with Full Showcase... " _
            run_demo all
            echo -e "${C_GREEN}Tour complete!${C_RESET}"
            ;;

        *)
            echo -e "${C_RED}Unknown demo mode: '${mode}'${C_RESET}"
            echo "Available modes: single, master, stack, floating, fullscreen, workspaces, all, tour"
            ;;
    esac
}

show_keybindings() {
    cat << EOF
${C_BOLD}${C_YELLOW}tilo Default Keybindings inside Xephyr:${C_RESET}
  ${C_CYAN}Mod4 + Return${C_RESET}           Spawn terminal (xterm / alacritty / kitty)
  ${C_CYAN}Mod4 + d${C_RESET}                Toggle Application Launcher (search & spawn apps)
  ${C_CYAN}Mod4 + Shift + Space${C_RESET}    Toggle Tiled <-> Floating mode for focused window
  ${C_CYAN}Mod4 + f${C_RESET}                Toggle Fullscreen mode for focused window
  ${C_CYAN}Mod4 + j / k${C_RESET}            Focus Next / Previous window (Down / Up in stack)
  ${C_CYAN}Mod4 + h / l${C_RESET}            Focus Left (Master) / Right (Stack)
  ${C_CYAN}Mod4 + Shift + j / k${C_RESET}    Swap focused window Down / Up in stack
  ${C_CYAN}Mod4 + Shift + h / l${C_RESET}    Swap focused window Left / Right (Master <-> Stack)
  ${C_CYAN}Mod4 + 1 .. 9${C_RESET}           Switch to Workspace 1..9
  ${C_CYAN}Mod4 + Shift + 1 .. 9${C_RESET}   Move focused window to Workspace 1..9
  ${C_CYAN}Mod4 + Shift + q${C_RESET}        Close focused window (graceful ICCCM delete)
  ${C_CYAN}Mod4 + Left Drag${C_RESET}        Move floating window
  ${C_CYAN}Mod4 + Right Drag${C_RESET}       Resize floating window

${C_YELLOW}Tips for Xephyr Testing:${C_RESET}
  - If your desktop intercepts Super/Mod4, press ${C_CYAN}Ctrl + Shift${C_RESET} inside Xephyr
    to capture/release the keyboard and mouse from your host OS.
  - Or type ${C_CYAN}launcher${C_RESET} in the interactive terminal prompt below!
EOF
}

show_session_info() {
    echo -e "${C_BOLD}Xephyr Session Info:${C_RESET}"
    echo -e "  Display    : ${C_GREEN}${XEPHYR_DISPLAY}${C_RESET}"
    echo -e "  Resolution : ${C_GREEN}${SCREEN_SIZE}${C_RESET} (Preset: ${SCREEN_PRESET})"
    echo -e "  Xephyr PID : ${XEPHYR_PID}"
    echo -e "  tilo PID   : ${TILO_PID}"
    echo -e "  Client PIDs: ${#CLIENT_PIDS[@]} active (${CLIENT_PIDS[*]:-none})"
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
echo -e "${C_BOLD}Interactive Prompt Commands:${C_RESET}"
echo -e "  ${C_CYAN}launcher / app${C_RESET}   Toggle tilo integrated application launcher"
echo -e "  ${C_CYAN}demo <mode>${C_RESET}      Run demo: ${C_YELLOW}single, master, stack, floating, fullscreen, workspaces, all, tour${C_RESET}"
echo -e "  ${C_CYAN}term / xterm${C_RESET}     Spawn terminal in Xephyr"
echo -e "  ${C_CYAN}clock / calc / eyes${C_RESET} Spawn sample app in Xephyr"
echo -e "  ${C_CYAN}launch <cmd>${C_RESET}     Run arbitrary command in Xephyr (e.g. launch firefox, launch htop)"
echo -e "  ${C_CYAN}<any-command>${C_RESET}    Directly launch any system command into Xephyr"
echo -e "  ${C_CYAN}clean / clear${C_RESET}    Close all demo client windows"
echo -e "  ${C_CYAN}reload${C_RESET}           Reload tilo config (SIGHUP)"
echo -e "  ${C_CYAN}keys${C_RESET}             Show tilo default keybindings"
echo -e "  ${C_CYAN}info / status${C_RESET}    Show active session info & clients"
echo -e "  ${C_CYAN}help / ?${C_RESET}         Show this commands list"
echo -e "  ${C_CYAN}quit / exit / q${C_RESET}  Shut down Xephyr session and exit"
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

    PROMPT_LABEL="${C_BOLD}${C_CYAN}tilo-xephyr [${SCREEN_PRESET} ${XEPHYR_DISPLAY}]>${C_RESET} "
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
        q|quit|exit)
            echo -e "${C_GREEN}Shutting down Xephyr session...${C_RESET}"
            break
            ;;

        help|\?)
            echo -e "${C_BOLD}Commands:${C_RESET}"
            echo -e "  ${C_CYAN}launcher / app${C_RESET}     Toggle integrated application launcher"
            echo -e "  ${C_CYAN}demo <single|master|stack|floating|fullscreen|workspaces|all|tour>${C_RESET}"
            echo -e "  ${C_CYAN}term / xterm${C_RESET}       Spawn terminal"
            echo -e "  ${C_CYAN}clock / xclock${C_RESET}     Spawn clock"
            echo -e "  ${C_CYAN}calc / xcalc${C_RESET}       Spawn calculator"
            echo -e "  ${C_CYAN}eyes / xeyes${C_RESET}       Spawn xeyes"
            echo -e "  ${C_CYAN}launch <cmd>${C_RESET}       Launch any command with DISPLAY=${XEPHYR_DISPLAY}"
            echo -e "  ${C_CYAN}clean / clear${C_RESET}      Close all demo clients"
            echo -e "  ${C_CYAN}reload${C_RESET}             Reload tilo config (SIGHUP)"
            echo -e "  ${C_CYAN}keys${C_RESET}               Show tilo keybindings"
            echo -e "  ${C_CYAN}info / status${C_RESET}      Show session status"
            echo -e "  ${C_CYAN}quit / exit / q${C_RESET}    Quit"
            ;;

        launcher|app|dmenu|rofi)
            if [[ -n "${TILO_PID}" ]] && kill -0 "${TILO_PID}" 2>/dev/null; then
                kill -USR1 "${TILO_PID}"
                echo -e "${C_GREEN}  [+] Toggled application launcher (SIGUSR1 -> tilo PID ${TILO_PID})${C_RESET}"
            else
                echo -e "${C_RED}  [-] tilo is not running.${C_RESET}"
            fi
            ;;

        reload)
            if [[ -n "${TILO_PID}" ]] && kill -0 "${TILO_PID}" 2>/dev/null; then
                kill -HUP "${TILO_PID}"
                echo -e "${C_GREEN}  [+] Sent SIGHUP to tilo PID ${TILO_PID} to reload configuration.${C_RESET}"
            else
                echo -e "${C_RED}  [-] tilo is not running.${C_RESET}"
            fi
            ;;

        demo)
            if [[ -n "${args}" ]]; then
                run_demo "${args}"
            else
                echo -e "Please specify demo type: ${C_CYAN}single, master, stack, floating, fullscreen, workspaces, all, tour${C_RESET}"
            fi
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
        all)
            run_demo all
            ;;
        tour|cycle)
            run_demo tour
            ;;

        term|xterm)
            if command -v xterm >/dev/null 2>&1; then
                launch_client xterm
            elif command -v alacritty >/dev/null 2>&1; then
                launch_client alacritty
            elif command -v kitty >/dev/null 2>&1; then
                launch_client kitty
            else
                echo -e "${C_RED}No standard terminal found (xterm, alacritty, kitty).${C_RESET}"
            fi
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
