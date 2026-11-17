# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.8.0] - 2026-11-18

### Added

- Native `megatemp-gui` desktop app built with egui/eframe: Overview, Create,
  Keep alive, Cloud, Accounts and Settings screens, keyboard shortcuts,
  toasts, a quick guide, and account selection for cloud operations.
- `--isolate` now runs each parallel worker in its own process.
- Chromium auto-download from Chrome for Testing.
- `--proxy-url` rotating proxy manager, verbose diagnostics, upload progress,
  and a Python-style health dashboard with storage bars and last-login ages.
- GitHub Actions CI and cross-platform release builds.

### Changed

- Binaries are named `megatemp` and `megatemp-gui` (the old `megatem` name is gone).
- Windows executables embed the MegaTemp logo; the GUI uses a crisp transparent
  SVG logo in the top bar and hides its console window.
- GUI windows start maximized and every screen scrolls.
- README, wiki, issue templates and repository metadata added for the public release.

### Fixed

- MEGA v2 session handling, legacy `csid` decryption, AES-128/192/256 key
  dispatch, node attribute keys, and upload handles.
- Parallel worker output is grouped per account instead of interleaved.
- Live keepalive reports accounts as alive instead of incorrectly marking them dead.

[1.8.0]: https://github.com/SchooiCodes/MegaTemp_Rust/releases/tag/v1.8.0
