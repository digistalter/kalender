#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Starts the installed app on a virtual screen and checks that it is still
# running after a few seconds (no crash at start).
set -eu
Xvfb :99 -screen 0 1280x800x24 >/dev/null 2>&1 &
xvfb=$!
sleep 2
status=0
DISPLAY=:99 GDK_BACKEND=x11 dbus-run-session -- timeout 20 kalender || status=$?
kill "$xvfb" 2>/dev/null || true
if [ "$status" -ne 124 ]; then
  echo "kalender stopped early (exit $status)" >&2
  exit 1
fi
echo "kalender started and kept running"
