# Tilo Window Manager Makefile
# Minimal Rust X11 Tiling Window Manager

NAME := tilo
VERSION := $(shell tr -d '[:space:]' < VERSION)
PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
CARGO ?= cargo
TARGET_DIR := target
SCRIPTS_DIR := scripts

.PHONY: all debug release build test test-xephyr test-xephyr-release test-xephyr-layout test-layout test-layouts clean install uninstall version version-check bump-patch bump-minor bump-major set-version help

# Default target: build debug binary
all: debug

# Build debug target
debug:
	@echo "==> Building $(NAME) [debug] v$(VERSION)..."
	$(CARGO) build

# Build release target with full optimizations
release:
	@echo "==> Building $(NAME) [release] v$(VERSION)..."
	$(CARGO) build --release
	@echo "Release binary built at $(TARGET_DIR)/release/$(NAME)"
	@ls -lh $(TARGET_DIR)/release/$(NAME)

# Alias for debug
build: debug

# Run cargo unit and integration tests
test:
	@echo "==> Running tests..."
	$(CARGO) test

# Run tilo inside a nested Xephyr X11 server for visual testing
test-xephyr: debug
	@echo "==> Starting nested Xephyr X11 test session..."
	$(SCRIPTS_DIR)/run-xephyr.sh

# Run tilo release build inside nested Xephyr
test-xephyr-release: release
	@echo "==> Starting nested Xephyr X11 test session (release)..."
	$(SCRIPTS_DIR)/run-xephyr.sh --release

# Run tilo inside Xephyr with demo windows (xclock, xcalc, xeyes) to test tiling layouts visually
test-xephyr-layout: debug
	@echo "==> Starting Xephyr layout test with sample windows..."
	$(SCRIPTS_DIR)/run-xephyr.sh --layout-demo

# Alias for test-xephyr-layout
test-layout: test-xephyr-layout

# Run automated integration tests for implemented layouts (master/stack and floating)
test-layouts:
	@echo "==> Running automated layout integration tests..."
	$(SCRIPTS_DIR)/run-xephyr.sh --test-layout

# Clean build artifacts
clean:
	@echo "==> Cleaning build artifacts..."
	$(CARGO) clean

# Install release binary
install: release
	@echo "==> Installing $(NAME) to $(DESTDIR)$(BINDIR)..."
	install -d $(DESTDIR)$(BINDIR)
	install -m 755 $(TARGET_DIR)/release/$(NAME) $(DESTDIR)$(BINDIR)/$(NAME)

# Uninstall binary
uninstall:
	@echo "==> Removing $(NAME) from $(DESTDIR)$(BINDIR)..."
	rm -f $(DESTDIR)$(BINDIR)/$(NAME)

# Display current version
version:
	@echo "$(NAME) v$(VERSION)"

# Verify VERSION file and Cargo.toml are synchronized
version-check:
	@$(SCRIPTS_DIR)/update-version.sh check

# Bump patch version (x.y.Z -> x.y.Z+1)
bump-patch:
	@$(SCRIPTS_DIR)/update-version.sh patch

# Bump minor version (x.Y.z -> x.Y+1.0)
bump-minor:
	@$(SCRIPTS_DIR)/update-version.sh minor

# Bump major version (X.y.z -> X+1.0.0)
bump-major:
	@$(SCRIPTS_DIR)/update-version.sh major

# Set an explicit version: make set-version V=0.2.0
set-version:
ifndef V
	$(error Please specify version: make set-version V=x.y.z)
endif
	@$(SCRIPTS_DIR)/update-version.sh $(V)

# Show help
help:
	@echo "Tilo Makefile targets:"
	@echo "  make              - Build debug executable (target/debug/tilo)"
	@echo "  make release      - Build optimized release executable (target/release/tilo)"
	@echo "  make test         - Run cargo unit and integration tests"
	@echo "  make test-xephyr  - Run debug build inside a nested Xephyr X11 display"
	@echo "  make test-xephyr-layout - Run tilo in Xephyr with demo windows to visually test tiling"
	@echo "  make test-layout  - Alias for test-xephyr-layout"
	@echo "  make clean        - Remove target/ directory"
	@echo "  make install      - Install release binary to \$$PREFIX/bin (default: /usr/local/bin)"
	@echo "  make uninstall    - Remove binary from \$$PREFIX/bin"
	@echo "  make version      - Print current version"
	@echo "  make version-check- Verify VERSION and Cargo.toml are synchronized"
	@echo "  make bump-patch   - Increment patch version"
	@echo "  make bump-minor   - Increment minor version"
	@echo "  make bump-major   - Increment major version"
	@echo "  make set-version V=x.y.z - Set an explicit version"
