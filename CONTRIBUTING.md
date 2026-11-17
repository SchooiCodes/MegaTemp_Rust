# Contributing

Thanks for helping improve MegaTemp.

## Development setup

You need a recent stable Rust toolchain and a C toolchain (MSVC build tools on
Windows). Build through `cargo` as usual:

```sh
cargo test
cargo build --release
```

If `link.exe` is not found on Windows, install the Visual Studio Build Tools
with the "Desktop development with C++" workload.

## Ground rules

- Keep compatibility with the Python original's `config.json`, `credentials/`
  and encryption formats. A change that requires migration needs a clear note.
- Keep the CLI and GUI in sync: both call the same shared library in `src/lib.rs`.
- Add or update tests for behaviour changes. The ignored live upload test can be
  run with `MEGATEM_LIVE_EMAIL` / `MEGATEM_LIVE_PASSWORD` set:
  `cargo test live_upload_roundtrip -- --ignored`.
- Never commit credentials, config files, browser profiles or build output.
- Follow the existing code style; run `cargo fmt` before opening a PR.

## Pull requests

Describe what changed and why, link any related issue, and include the commands
you ran to verify. CI must be green on Windows, Linux and macOS.

## Disclaimer

This project automates account creation on a third-party service. Do not use it
where that violates the service's terms or local law.
