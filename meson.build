# SPDX-License-Identifier: GPL-3.0-or-later
project(
  'kalender',
  version: '1.0.0',
  license: 'GPL-3.0-or-later',
  meson_version: '>= 0.62.0',
)

i18n = import('i18n')
gnome = import('gnome')

app_id = 'io.github.digistalter.Kalender'
gettext_package = meson.project_name()
prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
datadir = prefix / get_option('datadir')
localedir = prefix / get_option('localedir')

subdir('data')
subdir('po')

cargo = find_program('cargo')
cargo_options = [
  '--manifest-path', meson.project_source_root() / 'Cargo.toml',
  '--target-dir', meson.project_build_root() / 'target',
]
if get_option('buildtype') == 'debug'
  rust_target = 'debug'
else
  cargo_options += ['--release']
  rust_target = 'release'
endif
if get_option('offline')
  cargo_options += ['--offline']
endif

cargo_env = environment()
cargo_env.set('KALENDER_LOCALEDIR', localedir)

custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: meson.project_name(),
  console: true,
  install: true,
  install_dir: get_option('bindir'),
  env: cargo_env,
  command: [
    find_program('build-aux/cargo-build.sh'),
    cargo,
    meson.project_build_root() / 'target' / rust_target / meson.project_name(),
    '@OUTPUT@',
    cargo_options,
  ],
)

test(
  'cargo-test',
  cargo,
  args: ['test', cargo_options],
  env: cargo_env,
  timeout: 1800,
)

gnome.post_install(
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)
