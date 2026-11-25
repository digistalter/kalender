// SPDX-License-Identifier: GPL-3.0-or-later

pub const APP_ID: &str = "io.github.digistalter.Kalender";
pub const APP_NAME: &str = "Kalender";
pub const GETTEXT_PACKAGE: &str = "kalender";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LOCALEDIR: &str = match option_env!("KALENDER_LOCALEDIR") {
    Some(dir) => dir,
    None => "/usr/share/locale",
};
