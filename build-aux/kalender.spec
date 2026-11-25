# SPDX-License-Identifier: GPL-3.0-or-later
#
# Packs the files `meson install` put into %%{stage}. Built by
# build-aux/package-rpm.sh; dependencies on shared libraries are found by
# rpm itself, so the package works on Fedora and openSUSE Tumbleweed.
%global debug_package %{nil}
%global _build_id_links none

Name:           kalender
Version:        %{version}
Release:        1
Summary:        Simple calendar for appointments, meetings and tasks
License:        GPL-3.0-or-later
URL:            https://github.com/digistalter/kalender
Requires:       hicolor-icon-theme
Requires:       glib-networking
Recommends:     xdg-desktop-portal

%description
Kalender is a simple calendar for GNOME and KDE, made for planning at your
computer. Entries stay on your computer, without accounts; invitations go
out by e-mail. Tasks with subtasks, vacation days, birthdays, subscribed
calendars and a daily backup.

%install
cp -a %{stage}/. %{buildroot}/
install -Dm644 %{license_file} %{buildroot}%{_datadir}/licenses/kalender/LICENSE

%files
%license %{_datadir}/licenses/kalender/LICENSE
%{_bindir}/kalender
%{_datadir}/applications/io.github.digistalter.Kalender.desktop
%{_datadir}/metainfo/io.github.digistalter.Kalender.metainfo.xml
%{_datadir}/dbus-1/services/io.github.digistalter.Kalender.service
%{_datadir}/icons/hicolor/scalable/apps/io.github.digistalter.Kalender.svg
%{_datadir}/icons/hicolor/symbolic/apps/io.github.digistalter.Kalender-symbolic.svg
%{_datadir}/locale/*/LC_MESSAGES/kalender.mo
