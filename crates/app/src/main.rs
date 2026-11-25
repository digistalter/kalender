// SPDX-License-Identifier: GPL-3.0-or-later

mod application;
mod background;
mod backup;
mod config;
mod detail;
mod devtools;
mod draw;
mod editor;
mod fmt;
mod hooks;
mod i18n;
mod month;
mod notify;
mod ops;
mod prefs;
mod share;
mod state;
mod subscriptions;
mod week;
mod window;

use gettextrs::LocaleCategory;
use gtk::glib;

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    // The language chosen in Settings → General wins over the system.
    if let Some(language) = chosen_language() {
        // SAFETY: called before any other thread exists.
        unsafe { std::env::set_var("LANGUAGE", language) };
    }
    // SAFETY: called before any other thread exists.
    unsafe { gettextrs::setlocale(LocaleCategory::LcAll, "") };
    if let Err(error) = gettextrs::bindtextdomain(config::GETTEXT_PACKAGE, config::LOCALEDIR) {
        tracing::warn!("translations unavailable: {error}");
    }
    let _ = gettextrs::bind_textdomain_codeset(config::GETTEXT_PACKAGE, "UTF-8");
    let _ = gettextrs::textdomain(config::GETTEXT_PACKAGE);

    glib::set_application_name(config::APP_NAME);
    application::run()
}

/// "de" or "en" if the user picked a language, None to follow the system.
fn chosen_language() -> Option<String> {
    let store = kalender_core::store::Store::open(&application::database_path()).ok()?;
    let language = store.setting("language").ok().flatten()?;
    matches!(language.as_str(), "de" | "en").then_some(language)
}
