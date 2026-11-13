Name:           tilo
Version:        %{version}
Release:        1%{?dist}
Summary:        Minimal Rust X11 Tiling Window Manager

License:        MIT
URL:            https://github.com/lnoxsian/tilo
BuildArch:      %{target_arch}
AutoReqProv:    no

Recommends:     dejavu-sans-fonts
Suggests:       xorg-x11-server-Xephyr

%description
Tilo is a minimal, fast, dependency-light tiling window manager
for Linux/X11, written entirely in Rust. It provides an integrated
status bar, application launcher, workspace management, and ICCCM/EWMH support.

%prep

%build

%install
mkdir -p %{buildroot}
cp -r %{staging_dir}/* %{buildroot}/

%files
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
* Sat Nov 14 2026 Tilo Developers <dev@tilo.org> - %{version}-1
- Multi-architecture binary release package
