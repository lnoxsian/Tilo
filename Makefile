# Tilo Window Manager Makefile
# Minimal Rust X11 Tiling Window Manager

NAME := tilo
VERSION := $(shell tr -d '[:space:]' < VERSION)
PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
CARGO ?= cargo
TARGET_DIR := target
SCRIPTS_DIR := scripts

.PHONY: all debug release build test test-xephyr test-xephyr-release test-xephyr-layout test-layout test-layouts docker-build docker-compose install-deps clean install uninstall version version-check bump-patch bump-minor bump-major set-version help

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

# Build and export release binary using Docker BuildKit into dist/tilo
docker-build:
	@echo "==> Building and exporting $(NAME) release binary via Docker BuildKit..."
	@mkdir -p dist
	docker build --target export --output type=local,dest=dist .
	@echo "Release binary exported to dist/$(NAME)"
	@ls -lh dist/$(NAME)

# Build and export release binary using Docker Compose into dist/tilo
docker-compose:
	@echo "==> Building and exporting $(NAME) release binary via Docker Compose..."
	@mkdir -p dist
	docker compose up --build
	@docker compose down >/dev/null 2>&1 || true
	@echo "Release binary exported to dist/$(NAME)"
	@ls -lh dist/$(NAME)

# Alias for debug
build: debug

# Run cargo unit and integration tests
test:
	@echo "==> Running tests..."
	$(CARGO) test

# Run tilo inside a nested Xephyr X11 server for visual testing
test-xephyr: debug
	@echo "==> Starting nested Xephyr X11 test session..."
	$(SCRIPTS_DIR)/run-xephyr.sh --no-build

# Run tilo release build inside nested Xephyr
test-xephyr-release: release
	@echo "==> Starting nested Xephyr X11 test session (release)..."
	$(SCRIPTS_DIR)/run-xephyr.sh --release --no-build

# Run tilo inside Xephyr with demo windows to test implemented layouts visually
# Usage: make test-layout [LAYOUT=single|master|stack|floating|all]
test-xephyr-layout: debug
	@echo "==> Starting Xephyr layout demo..."
	$(SCRIPTS_DIR)/run-xephyr.sh --no-build --test-layout $(LAYOUT)

# Alias for test-xephyr-layout
test-layout: test-xephyr-layout

# Run automated integration tests for implemented layouts (master/stack and floating)
test-layouts:
	@echo "==> Running automated layout integration tests..."
	$(SCRIPTS_DIR)/run-xephyr.sh --automated-tests

# Install build and runtime dependencies (C libraries, Xephyr, and Rust)
install-deps:
	@$(SCRIPTS_DIR)/install_depnds.sh

# Clean build artifacts
clean:
	@echo "==> Cleaning build artifacts..."
	$(CARGO) clean
	rm -rf dist

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
	@echo "  make docker-build - Build and export release binary via Docker BuildKit (dist/tilo)"
	@echo "  make docker-compose - Build and export release binary via Docker Compose (dist/tilo)"
	@echo "  make test         - Run cargo unit and integration tests"
	@echo "  make test-xephyr  - Run debug build inside a nested Xephyr X11 display"
	@echo "  make test-xephyr-layout - Run tilo in Xephyr with demo windows to visually test tiling"
	@echo "  make test-layout  - Alias for test-xephyr-layout"
	@echo "  make install-deps - Install system dependencies and Rust toolchain"
	@echo "  make clean        - Remove target/ and dist/ directories"
	@echo "  make install      - Install release binary to \$$PREFIX/bin (default: /usr/local/bin)"
	@echo "  make uninstall    - Remove binary from \$$PREFIX/bin"
	@echo "  make version      - Print current version"
	@echo "  make version-check- Verify VERSION and Cargo.toml are synchronized"
	@echo "  make bump-patch   - Increment patch version"
	@echo "  make bump-minor   - Increment minor version"
	@echo "  make bump-major   - Increment major version"
	@echo "  make set-version V=x.y.z - Set an explicit version"
