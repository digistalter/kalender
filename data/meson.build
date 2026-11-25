# SPDX-License-Identifier: GPL-3.0-or-later

desktop_file = i18n.merge_file(
  input: configure_file(
    input: app_id + '.desktop.in.in',
    output: app_id + '.desktop.in',
    configuration: {'APP_ID': app_id},
  ),
  output: app_id + '.desktop',
  type: 'desktop',
  po_dir: meson.project_source_root() / 'po',
  install: true,
  install_dir: datadir / 'applications',
)
desktop_file_validate = find_program('desktop-file-validate', required: false)
if desktop_file_validate.found()
  test('validate-desktop', desktop_file_validate, args: [desktop_file])
endif

metainfo_file = i18n.merge_file(
  input: app_id + '.metainfo.xml.in',
  output: app_id + '.metainfo.xml',
  po_dir: meson.project_source_root() / 'po',
  install: true,
  install_dir: datadir / 'metainfo',
)
appstreamcli = find_program('appstreamcli', required: false)
if appstreamcli.found()
  test('validate-metainfo', appstreamcli, args: ['validate', '--no-net', '--explain', metainfo_file])
endif

configure_file(
  input: app_id + '.service.in',
  output: app_id + '.service',
  configuration: {'APP_ID': app_id, 'BINDIR': bindir},
  install: true,
  install_dir: datadir / 'dbus-1' / 'services',
)

icon_dir = meson.project_source_root() / 'design' / 'icon'
install_data(icon_dir / app_id + '.svg', install_dir: datadir / 'icons' / 'hicolor' / 'scalable' / 'apps')
install_data(icon_dir / app_id + '-symbolic.svg', install_dir: datadir / 'icons' / 'hicolor' / 'symbolic' / 'apps')
