# Tilo Window Manager Makefile
# Minimal Rust X11 Tiling Window Manager

NAME := tilo
VERSION := $(shell tr -d '[:space:]' < VERSION)
CARGO ?= cargo
TARGET_DIR := target
SCRIPTS_DIR := scripts
ASSETS_DIR := assets

# Packaging and installation paths (GNU / FHS conventions)
DESTDIR ?=
PREFIX ?= /usr/local
EXEC_PREFIX ?= $(PREFIX)
BINDIR ?= $(EXEC_PREFIX)/bin
DATAROOTDIR ?= $(PREFIX)/share
DATADIR ?= $(DATAROOTDIR)
SYSCONFDIR ?= /etc
XSESSIONDIR ?= $(DATADIR)/xsessions
APPLICATIONSDIR ?= $(DATADIR)/applications
ICONDIR ?= $(DATADIR)/icons/hicolor
PIXMAPDIR ?= $(DATADIR)/pixmaps
DOCDIR ?= $(DATADIR)/doc/$(NAME)
MANDIR ?= $(DATAROOTDIR)/man
MAN1DIR ?= $(MANDIR)/man1

# Install utilities
INSTALL ?= install
INSTALL_PROGRAM ?= $(INSTALL) -m 755
INSTALL_DATA ?= $(INSTALL) -m 644
INSTALL_DIR ?= $(INSTALL) -d -m 755

.PHONY: all debug release build test test-xephyr test-xephyr-release test-xephyr-layout test-layout test-layouts docker-build docker-compose install-deps clean install install-bin install-xsession install-desktop install-icons install-docs validate-desktop uninstall version version-check bump-patch bump-minor bump-major set-version help

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

# Install all components (binary, xsession desktop entry, icons, documentation)
install: install-bin install-xsession install-icons install-docs
	@echo "==> $(NAME) v$(VERSION) successfully installed to $(DESTDIR)$(PREFIX)"

# Install release binary only
install-bin: release
	@echo "==> Installing binary to $(DESTDIR)$(BINDIR)..."
	$(INSTALL_DIR) $(DESTDIR)$(BINDIR)
	$(INSTALL_PROGRAM) $(TARGET_DIR)/release/$(NAME) $(DESTDIR)$(BINDIR)/$(NAME)

# Install XSession desktop entry for display managers (GDM, LightDM, SDDM, Ly, etc.)
install-xsession:
	@echo "==> Installing XSession desktop entry to $(DESTDIR)$(XSESSIONDIR)..."
	$(INSTALL_DIR) $(DESTDIR)$(XSESSIONDIR)
	$(INSTALL_DATA) $(ASSETS_DIR)/$(NAME).desktop $(DESTDIR)$(XSESSIONDIR)/$(NAME).desktop

# Install application desktop entry (optional/convenience)
install-desktop:
	@echo "==> Installing Application desktop entry to $(DESTDIR)$(APPLICATIONSDIR)..."
	$(INSTALL_DIR) $(DESTDIR)$(APPLICATIONSDIR)
	$(INSTALL_DATA) $(ASSETS_DIR)/$(NAME).desktop $(DESTDIR)$(APPLICATIONSDIR)/$(NAME).desktop

# Install icons (scalable SVG and standard pixmaps)
install-icons:
	@echo "==> Installing icons to $(DESTDIR)$(ICONDIR) and $(DESTDIR)$(PIXMAPDIR)..."
	$(INSTALL_DIR) $(DESTDIR)$(ICONDIR)/scalable/apps
	$(INSTALL_DATA) $(ASSETS_DIR)/svg/tilo_logo.svg $(DESTDIR)$(ICONDIR)/scalable/apps/$(NAME).svg
	$(INSTALL_DIR) $(DESTDIR)$(PIXMAPDIR)
	$(INSTALL_DATA) $(ASSETS_DIR)/svg/tilo_logo.svg $(DESTDIR)$(PIXMAPDIR)/$(NAME).svg
	@if [ -f "$(ASSETS_DIR)/png/tilo_dark_transparent.png" ]; then \
		$(INSTALL_DATA) $(ASSETS_DIR)/png/tilo_dark_transparent.png $(DESTDIR)$(PIXMAPDIR)/$(NAME).png; \
	fi

# Install documentation and license
install-docs:
	@echo "==> Installing documentation to $(DESTDIR)$(DOCDIR)..."
	$(INSTALL_DIR) $(DESTDIR)$(DOCDIR)
	@if [ -f LICENSE ]; then $(INSTALL_DATA) LICENSE $(DESTDIR)$(DOCDIR)/; fi
	@if [ -f README.md ]; then $(INSTALL_DATA) README.md $(DESTDIR)$(DOCDIR)/; fi

# Validate desktop file syntax
validate-desktop:
	@echo "==> Validating $(ASSETS_DIR)/$(NAME).desktop..."
	@if command -v desktop-file-validate >/dev/null 2>&1; then \
		desktop-file-validate $(ASSETS_DIR)/$(NAME).desktop && echo "Desktop file is valid."; \
	else \
		echo "desktop-file-validate not found, skipping validation."; \
	fi

# Uninstall all installed files
uninstall:
	@echo "==> Removing $(NAME) from $(DESTDIR)$(PREFIX)..."
	rm -f $(DESTDIR)$(BINDIR)/$(NAME)
	rm -f $(DESTDIR)$(XSESSIONDIR)/$(NAME).desktop
	rm -f $(DESTDIR)$(APPLICATIONSDIR)/$(NAME).desktop
	rm -f $(DESTDIR)$(ICONDIR)/scalable/apps/$(NAME).svg
	rm -f $(DESTDIR)$(PIXMAPDIR)/$(NAME).svg
	rm -f $(DESTDIR)$(PIXMAPDIR)/$(NAME).png
	rm -rf $(DESTDIR)$(DOCDIR)
	@echo "==> Uninstall complete."

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
	@echo "  make                  - Build debug executable (target/debug/tilo)"
	@echo "  make release          - Build optimized release executable (target/release/tilo)"
	@echo "  make docker-build     - Build and export release binary via Docker BuildKit (dist/tilo)"
	@echo "  make docker-compose   - Build and export release binary via Docker Compose (dist/tilo)"
	@echo "  make test             - Run cargo unit and integration tests"
	@echo "  make test-xephyr      - Run debug build inside a nested Xephyr X11 display"
	@echo "  make test-xephyr-layout - Run tilo in Xephyr with demo windows to visually test tiling"
	@echo "  make test-layout      - Alias for test-xephyr-layout"
	@echo "  make install-deps     - Install system dependencies and Rust toolchain"
	@echo "  make clean            - Remove target/ and dist/ directories"
	@echo "  make install          - Install all components (binary, xsession, icons, docs)"
	@echo "  make install-bin      - Install release binary to \$$BINDIR"
	@echo "  make install-xsession - Install xsession desktop entry to \$$XSESSIONDIR"
	@echo "  make install-desktop  - Install desktop entry to \$$APPLICATIONSDIR"
	@echo "  make install-icons    - Install application icons"
	@echo "  make install-docs     - Install documentation and license to \$$DOCDIR"
	@echo "  make validate-desktop - Validate assets/tilo.desktop with desktop-file-validate"
	@echo "  make uninstall        - Remove all installed components"
	@echo "  make version          - Print current version"
	@echo "  make version-check    - Verify VERSION and Cargo.toml are synchronized"
	@echo "  make bump-patch       - Increment patch version"
	@echo "  make bump-minor       - Increment minor version"
	@echo "  make bump-major       - Increment major version"
	@echo "  make set-version V=x.y.z - Set an explicit version"
	@echo ""
	@echo "Installation path variables (override with VAR=...):"
	@echo "  PREFIX          (default: /usr/local, use /usr for distribution packaging)"
	@echo "  DESTDIR         (default: empty, staging directory for package builders)"
	@echo "  BINDIR          (default: \$$PREFIX/bin)"
	@echo "  DATADIR         (default: \$$PREFIX/share)"
	@echo "  XSESSIONDIR     (default: \$$DATADIR/xsessions)"
	@echo "  APPLICATIONSDIR (default: \$$DATADIR/applications)"
	@echo "  ICONDIR         (default: \$$DATADIR/icons/hicolor)"
	@echo "  PIXMAPDIR       (default: \$$DATADIR/pixmaps)"
	@echo "  DOCDIR          (default: \$$DATADIR/doc/tilo)"
	@echo "  MANDIR          (default: \$$DATADIR/man)"
