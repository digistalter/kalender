// SPDX-License-Identifier: GPL-3.0-or-later

//! Daily backup: once a day a complete copy of all data goes into a folder
//! (by default Documents/Kalender backups); the newest ten are kept.

use std::path::PathBuf;
use std::rc::Rc;

use anyhow::Result;
use gtk::glib;

use crate::i18n::tr;
use crate::state::{State, now, today};
use crate::window::UiRc;

const KEEP: usize = 10;
const PREFIX: &str = "kalender-";

/// The folder backups go to.
pub fn folder(st: &State) -> PathBuf {
    if !st.settings.backup_dir.trim().is_empty() {
        return PathBuf::from(st.settings.backup_dir.trim());
    }
    glib::user_special_dir(glib::UserDirectory::Documents)
        .unwrap_or_else(glib::home_dir)
        .join(tr("Kalender backups"))
}

/// Writes today's backup unless it exists already (`force`: a new one in any case),
/// then removes old ones.
/// Returns the file written, if any.
pub fn run(st: &State, force: bool) -> Result<Option<PathBuf>> {
    let dir = folder(st);
    std::fs::create_dir_all(&dir)?;
    // The daily copy is named by date; one made by hand also gets the time,
    // so it never replaces an existing copy.
    let file = if force {
        dir.join(format!(
            "{PREFIX}{}.sqlite",
            now().format("%Y-%m-%d-%H%M%S")
        ))
    } else {
        dir.join(format!("{PREFIX}{}.sqlite", today().format("%Y-%m-%d")))
    };
    if file.exists() {
        return Ok(None);
    }
    st.backup_to(&file)?;
    // Keep the newest few (names sort by date).
    let mut old: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(PREFIX) && n.ends_with(".sqlite"))
        })
        .collect();
    old.sort();
    if old.len() > KEEP {
        for p in &old[..old.len() - KEEP] {
            let _ = std::fs::remove_file(p);
        }
    }
    Ok(Some(file))
}

/// Checks shortly after the start and then every hour whether today's backup is due.
pub fn start(ui: &UiRc) {
    // Demo data is never backed up.
    if std::env::var_os("KALENDER_DEMO").is_some() {
        return;
    }
    let check = |ui: &UiRc| {
        let st = ui.state.borrow();
        if st.settings.backup
            && let Err(e) = run(&st, false)
        {
            tracing::warn!("backup: {e:#}");
        }
    };
    let weak = Rc::downgrade(ui);
    glib::timeout_add_seconds_local_once(10, move || {
        if let Some(u) = weak.upgrade() {
            check(&u);
        }
    });
    let weak = Rc::downgrade(ui);
    glib::timeout_add_seconds_local(3600, move || {
        let Some(u) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        check(&u);
        glib::ControlFlow::Continue
    });
}
