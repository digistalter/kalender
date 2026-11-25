// SPDX-License-Identifier: GPL-3.0-or-later

//! Starting hidden at login, so reminders arrive as system notifications
//! even when the window was never opened.

use gtk::glib;

use crate::config;
use crate::i18n::tr;

/// Command line flag: start without showing the window.
pub const FLAG: &str = "--background";

/// Registers or removes the start at login, following "Run in the background".
pub fn sync(enabled: bool) {
    if std::path::Path::new("/.flatpak-info").exists() {
        glib::MainContext::default().spawn_local(async move {
            let reason = tr("Show reminders after login");
            let result = async {
                ashpd::desktop::background::Background::request()
                    .reason(reason.as_str())
                    .auto_start(enabled)
                    .command(["kalender", FLAG])
                    .dbus_activatable(false)
                    .send()
                    .await?
                    .response()
            }
            .await;
            if let Err(e) = result {
                tracing::warn!("background portal: {e}");
            }
        });
        return;
    }
    // Only installed builds register themselves, never a development binary.
    if option_env!("KALENDER_LOCALEDIR").is_none() {
        return;
    }
    let path = glib::user_config_dir()
        .join("autostart")
        .join(format!("{}.desktop", config::APP_ID));
    let result = if enabled {
        let exe = std::env::current_exe().unwrap_or_else(|_| "kalender".into());
        let file = format!(
            "[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec={} {FLAG}\nIcon={}\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
            config::APP_NAME,
            tr("Show reminders after login"),
            exe.display(),
            config::APP_ID,
        );
        std::fs::create_dir_all(path.parent().expect("autostart dir"))
            .and_then(|()| std::fs::write(&path, file))
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    };
    if let Err(e) = result {
        tracing::warn!("autostart {}: {e}", path.display());
    }
}
