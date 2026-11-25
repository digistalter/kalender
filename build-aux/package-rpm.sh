#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Builds the .rpm package (Fedora, openSUSE Tumbleweed) from the source tree.
# Usage: build-aux/package-rpm.sh OUTDIR
set -eu
out="$(realpath -m "$1")"
mkdir -p "$out"
version="$(sed -n "s/^  version: '\(.*\)',$/\1/p" meson.build)"
stage="$(mktemp -d)"
top="$(mktemp -d)"

meson setup build-rpm --prefix=/usr --buildtype=release
meson compile -C build-rpm
DESTDIR="$stage" meson install -C build-rpm --no-rebuild

rpmbuild -bb \
  --define "_topdir $top" \
  --define "stage $stage" \
  --define "license_file $PWD/LICENSE" \
  --define "version $version" \
  build-aux/kalender.spec
find "$top/RPMS" -name '*.rpm' -exec cp {} "$out/" \;
rm -rf "$stage" "$top"
