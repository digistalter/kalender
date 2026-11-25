#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Builds the .deb package (Ubuntu, Debian, Linux Mint) from the source tree.
# Dependencies on shared libraries are found by dpkg-shlibdeps, so the
# package asks for exactly the libraries the program uses.
# Usage: build-aux/package-deb.sh OUTDIR
set -eu
out="$(realpath -m "$1")"
mkdir -p "$out"
version="$(sed -n "s/^  version: '\(.*\)',$/\1/p" meson.build)"
root="$(mktemp -d)"
work="$(mktemp -d)"

meson setup build-deb --prefix=/usr --buildtype=release
meson compile -C build-deb
DESTDIR="$root" meson install -C build-deb --no-rebuild

mkdir -p "$work/debian"
printf 'Source: kalender\n\nPackage: kalender\nArchitecture: any\n' > "$work/debian/control"
depends="$(cd "$work" && dpkg-shlibdeps -O -e"$root/usr/bin/kalender" 2>/dev/null \
  | sed -n 's/^shlibs:Depends=//p')"
arch="$(dpkg --print-architecture)"
size="$(du -sk "$root" | cut -f1)"

mkdir -p "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<CONTROL
Package: kalender
Version: $version
Architecture: $arch
Maintainer: The Kalender developers <308797711+digistalter@users.noreply.github.com>
Installed-Size: $size
Depends: $depends, glib-networking
Recommends: xdg-desktop-portal
Section: utils
Priority: optional
Homepage: https://github.com/digistalter/kalender
Description: Simple calendar for appointments, meetings and tasks
 Kalender is a simple calendar for GNOME and KDE, made for planning at your
 computer. Entries stay on your computer, without accounts; invitations go
 out by e-mail. Tasks with subtasks, vacation days, birthdays, subscribed
 calendars and a daily backup.
CONTROL

dpkg-deb --root-owner-group --build "$root" "$out/kalender_${version}_${arch}.deb"
rm -rf "$root" "$work"
