# MegaTemp (Rust)

MegaTemp is a bulk MEGA account creator and manager. This repository contains the
Rust port: a single `megatemp` binary that is both a native GUI and a
scriptable CLI, sharing one library, one `config.json` and one credential store.

- [Source code](https://github.com/SchooiCodes/MegaTemp_Rust)
- [Releases](https://github.com/SchooiCodes/MegaTemp_Rust/releases)
- [Original Python project](https://github.com/SchooiCodes/MegaTemp)

> **Disclaimer:** this tool automates account creation on a third-party service.
> Use it only where that is allowed. It is not affiliated with or endorsed by MEGA.

## Quick start

1. Download the archive for your platform from
   [Releases](https://github.com/SchooiCodes/MegaTemp_Rust/releases) and extract it.
2. Run `megatemp` for the desktop app; add CLI flags for scripted work, or use
   `megatemp --menu` for the classic terminal menu. Standalone binaries and
   archives are both attached to each release.
3. The first account needs a Chromium-based browser (Chrome, Edge, Brave or
   Chromium). If none is detected, set the path in Settings or pass `--browser`.

From source:

```sh
cargo test
cargo build --release
```

See [CLI](CLI), [GUI](GUI), [Compatibility](Compatibility) and [FAQ](FAQ) for details.
