# Kalender entwickeln

Wie man Kalender aus dem Quelltext baut, testet und ausprobiert. Für Nutzer steht alles Weitere in der [README](../README.de.md).

## Aufbau

| Pfad | Inhalt |
|---|---|
| [`crates/sync/`](../crates/sync/) | Regeln für den Abgleich zwischen Geräten (logische Uhr, Zusammenführen je Feld), ohne Speicher und Netz, mit Tests |
| [`crates/link/`](../crates/link/) | Verbindung zwischen Geräten im Heimnetz: Finden (mDNS), Koppeln (QR-Code/Zahl, Prüfwörter), verschlüsselter Abgleich (Noise), mit Tests |
| [`crates/core/`](../crates/core/) | Logik ohne Oberfläche, mit Tests |
| [`crates/app/`](../crates/app/) | Die App mit GTK 4 und libadwaita |
| [`data/`](../data/) | Starter-Eintrag, Metainfo, D-Bus-Dienst, Bildschirmfotos |
| [`po/`](../po/) | Übersetzungen |
| [`build-aux/flatpak/`](../build-aux/flatpak/) | Flatpak-Rezept |
| [`design/icon/`](../design/icon/) | App-Icon (farbig und einfarbig) |

`crates/core`:

| Datei | Aufgabe |
|---|---|
| `model.rs` | Einträge, Arten, Serien, Routinen, Abo-Quellen |
| `recur.rs` | Serien in einzelne Termine auflösen |
| `store.rs` | Speicher (SQLite, je Objekt ein JSON-Datensatz), Sicherung und Wiederherstellen |
| `devices.rs` | Eigener Schlüssel dieses Geräts und die gekoppelten Geräte |
| `journal.rs` | Änderungsprotokoll für den Abgleich: jede Änderung je Feld, mit Kennungen, die auf allen Geräten gleich sind |
| `ics.rs` | iCalendar lesen und schreiben (Einladung, Antwort, Absage, Import, Export) |
| `subscription.rs` | Abonnierte Kalender: Links umwandeln, Einträge bilden |
| `vacation.rs` | Urlaubstage zählen, Alter |
| `holidays.rs` | Feiertage je Bundesland |
| `layout.rs` | Gleichzeitige Einträge nebeneinander verteilen |

`crates/app` (Auswahl):

| Datei | Aufgabe |
|---|---|
| `window.rs` | Hauptfenster, Seitenleiste, Kopfleiste, Aktionen |
| `week.rs`, `month.rs`, `draw.rs` | Woche und Monat als eigene Zeichenflächen, Symbole |
| `detail.rs`, `editor.rs`, `ops.rs` | Kurzansicht, Eintragsfenster, Verschieben/Kopieren/Löschen mit Rückgängig |
| `share.rs` | Einladungen per E-Mail, .ics öffnen, Import und Export |
| `subscriptions.rs` | Abos laden (libsoup) und alle 30 Minuten aktualisieren |
| `backup.rs` | Tägliche Sicherung |
| `notify.rs`, `background.rs` | Erinnerungen, Start bei der Anmeldung |
| `prefs.rs` | Einstellungen |
| `devtools.rs` | Beispieldaten und Bildschirmfotos (nur über Umgebungsvariablen) |

## Selbst bauen und ausprobieren

Zuerst die nötigen Pakete installieren.

Ubuntu:

```bash
sudo apt install cargo rustc meson ninja-build gettext libgtk-4-dev libadwaita-1-dev libsoup-3.0-dev glib-networking desktop-file-utils appstream libglib2.0-dev-bin
```

Fedora:

```bash
sudo dnf install cargo rust meson gettext gtk4-devel libadwaita-devel libsoup3-devel glib-networking desktop-file-utils appstream glib2-devel
```

Rust muss mindestens Version 1.89 haben (`rustc --version`), libadwaita mindestens 1.5.

Dann bauen und für den eigenen Benutzer installieren (landet in `~/.local`, kein Administrator nötig):

```bash
meson setup builddir --prefix="$HOME/.local" --buildtype=release
meson install -C builddir
```

Danach erscheint „Kalender“ in der App-Übersicht, und Kalenderdateien (`.ics`) lassen sich per Doppelklick im Kalender öffnen. Beim ersten Installieren startet die App aus der App-Übersicht erst nach einem Ab- und Anmelden, oder nachdem das System die Liste der Apps neu eingelesen hat:

```bash
gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.ReloadConfig
```

Läuft der Kalender im Hintergrund weiter, nach einem neuen Installieren einmal beenden (Menü → Beenden oder <kbd>Strg</kbd>+<kbd>Q</kbd>), damit der neue Stand startet.

Wieder entfernen:

```bash
ninja -C builddir uninstall
rm -f ~/.config/autostart/io.github.digistalter.Kalender.desktop
```

Die Daten liegen in `~/.local/share/kalender/kalender.sqlite`, die täglichen Sicherungen standardmäßig in `~/Dokumente/Kalender-Sicherungen`.

## Ausprobieren mit Beispieldaten

Ohne den eigenen Kalender anzufassen: ein eigener Datenordner und Beispieleinträge für die aktuelle Woche. Mit Beispieldaten wird nie gesichert.

```bash
XDG_DATA_HOME=/tmp/kalender-demo KALENDER_DEMO=1 cargo run
```

Hilfen für Bildschirmfotos:

| Variable | Wirkung |
|---|---|
| `KALENDER_NOW=2026-11-24T10:36` | feste Uhrzeit |
| `KALENDER_SHOT=bild.png` | Bild des Fensters speichern und beenden; Dialoge und Popover kommen als `bild.dialog.png` bzw. `bild.popover.png` dazu |
| `KALENDER_SHOT_VIEW=month` | Monat statt Woche |
| `KALENDER_SHOT_DARK=1` | dunkler Stil |
| `KALENDER_SHOT_SIZE=400x760` | Fenstergröße |
| `KALENDER_SHOT_ACTION=…` | vorher ausführen: `open:Titel`, `edit:Titel`, `ctx:Titel`, `dup:Titel`, `delete:Titel`, `new:Task` (Art auf Englisch), `search:Text`, `prefs`, `prefs:kinds`, `kinds`, `shortcuts`, `routines`, `subscribe:Link`, `notify:Titel`, `backup` |

Da die App nur einmal laufen kann, zwischen zwei Aufnahmen kurz warten und eine laufende Instanz vorher beenden.

## Abgleich zwischen zwei Programmen ausprobieren

Ein kleines Prüfwerkzeug koppelt zwei Kalender-Dateien und gleicht sie ab, auch zwischen zwei Rechnern im selben Netz:

```bash
cargo run -p kalender-core --example abgleich -- /tmp/PC.sqlite serve
```

Im zweiten Terminal mit dem angezeigten QR-Text (oder `pair IP:PORT ZAHL`), dann abgleichen:

```bash
cargo run -p kalender-core --example abgleich -- /tmp/Handy.sqlite pair "kalender:pair?…"
cargo run -p kalender-core --example abgleich -- /tmp/Handy.sqlite sync
```

Weitere Befehle: `add "Titel"`, `list`. Das Finden im Netz testet `cargo test -p kalender-link -- --ignored` (braucht ein Netz mit Multicast).

## Tests und Prüfungen

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
meson test -C builddir
```

`meson test` prüft zusätzlich Starter-Eintrag und Metainfo.

## Pakete und Veröffentlichung

`build-aux/package-deb.sh dist` und `build-aux/package-rpm.sh dist` bauen die Pakete, das Flatpak-Rezept liegt in `build-aux/flatpak/`. Auf GitHub baut [`packages.yml`](../.github/workflows/packages.yml) alle drei, installiert und startet sie einmal. Ein Tag wie `v1.0.1` hängt sie an ein neues Release.

Für eine neue Version: Versionsnummer in `meson.build`, `Cargo.toml` und im `<releases>`-Teil der Metainfo erhöhen, in den READMEs die Dateinamen anpassen, dann taggen.

## Übersetzung

Texte entstehen auf Englisch im Code (`tr`, `trf`, `trn`, `trcf`), die deutsche Übersetzung steht in [`po/de.po`](../po/de.po). Neue Quelldateien mit Texten gehören in [`po/POTFILES.in`](../po/POTFILES.in). Vorlage neu erzeugen und abgleichen:

```bash
ninja -C builddir kalender-pot
msgmerge --update po/de.po po/kalender.pot
```
