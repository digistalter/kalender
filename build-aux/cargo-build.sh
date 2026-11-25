#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Usage: cargo-build.sh CARGO BUILT_BINARY OUTPUT [cargo build options…]
set -eu
cargo="$1"
built="$2"
output="$3"
shift 3
"$cargo" build "$@"
cp "$built" "$output"
