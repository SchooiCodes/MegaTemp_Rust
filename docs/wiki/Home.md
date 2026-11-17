# MegaTemp (Rust)

MegaTemp is a bulk MEGA account creator and manager. This repository contains the
Rust port: a native GUI (`megatemp-gui`) and a scriptable CLI (`megatemp`) that
share one library, one `config.json` and one credential store.

- [Source code](https://github.com/SchooiCodes/MegaTemp_Rust)
- [Releases](https://github.com/SchooiCodes/MegaTemp_Rust/releases)
- [Original Python project](https://github.com/SchooiCodes/MegaTemp)

> **Disclaimer:** this tool automates account creation on a third-party service.
> Use it only where that is allowed. It is not affiliated with or endorsed by MEGA.

## Quick start

1. Download the archive for your platform from
   [Releases](https://github.com/SchooiCodes/MegaTemp_Rust/releases) and extract it.
2. Run `megatemp-gui` for the desktop app, or `megatemp` for the CLI/menu.
3. The first account needs a Chromium-based browser (Chrome, Edge, Brave or
   Chromium). If none is detected, set the path in Settings or pass `--browser`.

From source:

```sh
cargo test
cargo build --release
```

See [CLI](CLI), [GUI](GUI), [Compatibility](Compatibility) and [FAQ](FAQ) for details.
