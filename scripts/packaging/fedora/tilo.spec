Name:           tilo
Version:        0.2.5
Release:        1%{?dist}
Summary:        Minimal Rust X11 Tiling Window Manager

License:        MIT
URL:            https://github.com/lnoxsian/tilo
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  make
BuildRequires:  gcc
BuildRequires:  libX11-devel
BuildRequires:  libXrandr-devel
BuildRequires:  libxcb-devel

Requires:       libX11
Requires:       libXrandr
Requires:       libxcb
Recommends:     dejavu-sans-fonts
Suggests:       xorg-x11-server-Xephyr

%description
Tilo is a minimal, fast, dependency-light tiling window manager
for Linux/X11, written entirely in Rust. It provides an integrated
status bar, application launcher, workspace management, and ICCCM/EWMH support.

%prep
%autosetup

%build
make release

%install
%make_install PREFIX=%{_prefix}

%check
make test

%files
%license LICENSE
%doc README.md
%{_bindir}/tilo
%{_datadir}/xsessions/tilo.desktop
%dir %{_datadir}/slick-greeter
%dir %{_datadir}/slick-greeter/badges
%{_datadir}/slick-greeter/badges/tilo.svg
%{_datadir}/icons/hicolor/scalable/apps/tilo*.svg
%dir %{_datadir}/icons/hicolor/scalable/badges
%{_datadir}/icons/hicolor/scalable/badges/tilo*.svg
%{_datadir}/pixmaps/tilo.svg
%{_datadir}/pixmaps/tilo.png
%dir %{_datadir}/doc/tilo
%{_datadir}/doc/tilo/README.md
%{_datadir}/doc/tilo/LICENSE

%changelog
* Fri Oct 09 2026 Tilo Developers <dev@tilo.org> - 0.2.5-1
- Initial release for Fedora / RPM packaging
