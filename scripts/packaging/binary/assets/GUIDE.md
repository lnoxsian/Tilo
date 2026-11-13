# Tilo Window Manager - Installation & Dependencies Guide

This guide provides the minimal runtime dependencies and installation steps for running **Tilo** across major Linux distributions using the standalone binary release archive.

---

## 1. Minimal Runtime Dependencies

Tilo is a minimal, self-contained X11 window manager with built-in font rasterization (`fontdue`) and status bar/launcher. It does not require C GUI toolkits (no GTK, Qt, Cairo, or Pango).

To run Tilo, you only need:
- An **X11 Server** (`xorg` / `xorg-server`)
- **D-Bus** session bus
- A **TrueType / OpenType font** (e.g. DejaVu Sans Mono or Liberation Mono)
- A **Terminal Emulator** of your choice (e.g. `foot`, `alacritty`, `st`, `xterm`)

Install the minimal dependencies for your distribution:

### Alpine Linux
```sh
apk add xorg-server xinit font-dejavu dbus foot
```

### Arch Linux
```sh
sudo pacman -S xorg-server xorg-xinit ttf-dejavu dbus foot
```

### Void Linux
```sh
sudo xbps-install -S xorg-minimal xinit dejavu-fonts dbus foot
```

### Debian / Ubuntu
```sh
sudo apt update && sudo apt install -y xserver-xorg xinit fonts-dejavu-core dbus-x11 foot
```

### Fedora / RHEL
```sh
sudo dnf install -y xorg-x11-server-Xorg xorg-x11-xinit dejavu-sans-mono-fonts dbus-x11 foot
```

---

## 2. Installation

Extract the downloaded archive and run `install.sh`:

```sh
tar -xzf tilo-v*-*.tar.gz
cd tilo-v*-*
```

### System-wide Install (Recommended)
Installs to `/usr/local/bin` and `/usr/local/share`:
```sh
sudo ./install.sh
```

### User-local Install (No root required)
Installs to `~/.local/bin` and `~/.local/share`:
```sh
PREFIX=~/.local ./install.sh
```
*(Ensure `~/.local/bin` is in your `$PATH`)*.

---

## 3. Starting Tilo

### Option A: Via Display Manager (GDM, LightDM, SDDM, Ly)
`install.sh` installs the session file to `share/xsessions/tilo.desktop`.
Simply select **Tilo** on your login screen.

### Option B: Via `startx` / `~/.xinitrc`
Add Tilo to your `~/.xinitrc`:
```sh
cat << 'EOF' > ~/.xinitrc
#!/bin/sh
if [ -z "$DBUS_SESSION_BUS_ADDRESS" ]; then
    eval $(dbus-launch --sh-syntax --exit-with-session)
fi
exec tilo
EOF
chmod +x ~/.xinitrc
```

Then start your session:
```sh
startx
```

---

## 4. Uninstallation

To remove all installed files:
```sh
sudo ./install.sh --uninstall
```
Or for user-local installations:
```sh
PREFIX=~/.local ./install.sh --uninstall
```
