// SPDX-License-Identifier: GPL-3.0-or-later

fn main() {
    println!("cargo:rerun-if-env-changed=KALENDER_LOCALEDIR");
}
