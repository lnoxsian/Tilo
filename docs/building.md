# Building Tilo

This document covers everything needed to build, test, install, and release tilo.

---

## Prerequisites

### Required

| Tool / Library | Purpose | Install (Debian/Ubuntu) |
|---|---|---|
| Rust stable (edition 2024) | Compiler | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| Cargo | Package manager and build tool | Included with Rust |
| X11 development headers | `x11rb` XCB bindings | `libx11-dev libxrandr-dev libxcb1-dev libxcb-util-dev` |
| pkg-config | Build-time library detection | `pkg-config` |

### Font Requirements

| Package | Purpose | Install (Debian/Ubuntu) |
|---|---|---|
| TrueType / OpenType fonts | System fonts for UI rendering | `fonts-dejavu-core` or `fonts-liberation` |

> [!TIP]
> One-liner for Debian/Ubuntu:
> ```bash
> sudo apt install build-essential pkg-config \
>     libx11-dev libxrandr-dev libxcb1-dev fonts-dejavu-core
> ```

### Required for testing only

| Tool | Purpose | Install |
|---|---|---|
| Xephyr | Nested X11 server for isolated WM testing | `xserver-xephyr` |
| xterm / xclock / xcalc / xeyes | X11 test clients used by the harness | `x11-apps` |

---

## Build Targets

### Debug Build

```bash
cargo build
# or
make
```

Output: `target/debug/tilo`

The debug build includes debug symbols, assertions, and stack-overflow detection. Do not use for benchmarking.

### Release Build

```bash
cargo build --release
# or
make release
```

Output: `target/release/tilo`

The release profile applies maximum size optimizations:

```toml
[profile.release]
opt-level = "z"       # optimize for binary size
lto = true            # link-time optimization across crates
codegen-units = 1     # single codegen unit for best LTO
panic = "abort"       # no unwinding, smaller binary
strip = true          # strip debug symbols from final binary
```

### Dual-Engine Font Architecture

Tilo features a 100% pure Rust dual-engine font rendering pipeline:
1. **Modern TrueType / OpenType Engine (`fontdue`)**: Renders anti-aliased glyphs into an in-memory software framebuffer with SIMD rasterization.
2. **Zero-Allocation Core X11 Fallback (`is_core = true`)**: Automatically activates if `font fixed` or `font core:...` is configured, or if no font files are present on the host system, keeping memory under 2 MB RSS.

---

## Makefile Reference

The `Makefile` wraps common Cargo and script commands:

```bash
make              # Build debug binary (default)
make release      # Build optimized release binary
make build        # Alias for make (debug)
make test         # Run cargo unit and integration tests
make test-xephyr  # Run debug build inside a nested Xephyr display
make test-xephyr-release  # Run release build inside Xephyr
make test-layouts # Run automated Xephyr layout integration tests
make clean        # Remove the target/ directory
make install      # Install release binary to $PREFIX/bin (default /usr/local/bin)
make uninstall    # Remove binary from $PREFIX/bin
make version      # Print current version
make version-check # Verify VERSION and Cargo.toml are synchronized
make bump-patch   # Increment patch version (x.y.Z → x.y.Z+1)
make bump-minor   # Increment minor version (x.Y.z → x.Y+1.0)
make bump-major   # Increment major version (X.y.z → X+1.0.0)
make set-version V=x.y.z  # Set an explicit version
make help         # Show all targets
```

### Install Prefix

```bash
make install PREFIX=/usr           # installs to /usr/bin/tilo
make install PREFIX=~/.local       # installs to ~/.local/bin/tilo
make install DESTDIR=/tmp/staging  # for package maintainers
```

---

## Installation

After building the release binary:

```bash
sudo make install
# or manually:
sudo install -m 755 target/release/tilo /usr/local/bin/tilo
```

### Launching from a Display Manager

Add an entry in `/usr/share/xsessions/tilo.desktop`:

```ini
[Desktop Entry]
Name=tilo
Comment=Minimal X11 tiling window manager
Exec=tilo
Type=XSession
```

### Launching from ~/.xinitrc

```bash
exec tilo
```

---

## Version Management

The version is kept in two synchronized places:

- `VERSION` — plain text file (`0.2.1\n`)
- `Cargo.toml` — `version = "0.2.1"` in `[package]`

The `scripts/update-version.sh` script keeps both in sync and regenerates `Cargo.lock`.

### Checking Version Consistency

```bash
make version-check
# or directly:
./scripts/update-version.sh check
```

### Bumping Versions

```bash
make bump-patch     # 0.2.1 → 0.2.2
make bump-minor     # 0.2.1 → 0.3.0
make bump-major     # 0.2.1 → 1.0.0
make set-version V=0.5.0   # set explicitly
```

These commands update `VERSION`, `Cargo.toml`, and run `cargo check` to update `Cargo.lock`.

---

## Binary Size

The release build is optimized aggressively for size:

```bash
ls -lh target/release/tilo
```

Expected sizes:
- Optimized release build: **< 1 MB** (~945 KB stripped)

Strip manually if needed:
```bash
strip target/release/tilo
```

---

## Dependency Audit

The dependency tree is 100% pure Rust and zero-C-library:

```
tilo
├── fontdue        — Pure Rust TrueType/OpenType rasterizer (SIMD)
├── x11rb          — X11/XCB protocol bindings (with randr, cursor, resource_manager)
├── libc           — Linux syscall bindings (malloc_trim on Linux)
└── thiserror      — Error type derivation
```

Run a dependency audit:
```bash
cargo audit        # requires cargo-audit: cargo install cargo-audit
cargo tree         # visualize full dependency tree
```

---

## Cargo Commands Reference

```bash
cargo check                   # Fast type-check without producing a binary
cargo build                   # Debug build
cargo build --release         # Release build
cargo test                    # Run all unit and integration tests
cargo test --test layout      # Run only the layout integration tests
cargo test --test config      # Run only the config tests
cargo clean                   # Remove the target/ directory
cargo doc --open              # Generate and open Rust API documentation
```

---

## Troubleshooting Build Issues

| Problem | Solution |
|---|---|
| `xcb.h: No such file` | Install `libxcb1-dev` |
| `error[E0463]: can't find crate for std` | Ensure Rust stable is installed and up-to-date |
| Compilation error on edition 2024 features | Run `rustup update stable` to get latest Rust |
| `make: cargo: Command not found` | Set `CARGO` env var or add `~/.cargo/bin` to `PATH` |
