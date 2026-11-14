# MegaTemp (Rust)

A Rust port of [MegaTemp](https://github.com/SchooiCodes/MegaTemp), a tool for
bulk-creating and managing MEGA accounts.

The port is drop-in compatible with the original Python data files:

- `config.json` / `config-<profile>.json`
- `credentials/*.json`, `credentials/accounts.txt`, `accounts.csv`, `accounts.jsonl`
- encrypted credential fields (`ENC2:`, legacy `ENC:`, legacy `OBS:`)

## Build

```sh
cargo build --release
```

The binary is written to `target/release/megatem`.

## Usage

```sh
megatem --help
megatem --health
megatem -a 4 -sh
megatem --keepalive --prune -y
megatem --export-bitwarden --export-keepass
megatem --list-cloud
```

Running `megatem` without arguments opens the interactive menu.

## License

GPL-3.0-only. This is a derivative work of MegaTemp by SchooiCodes.
