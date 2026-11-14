# MegaTemp (Rust)

A Rust port of [MegaTemp](https://github.com/SchooiCodes/MegaTemp) — a tool for
bulk-creating and managing MEGA accounts.

The port keeps the behaviour and the on-disk formats of the Python original, so
it can be dropped into an existing installation without migrating anything.

## ⚠️ Disclaimer

This tool automates account creation on a third-party service. Use it only
where you are allowed to: creating accounts in bulk can violate MEGA's terms of
service, and doing so may get your IP address or accounts blocked. The authors
take no responsibility for how this software is used.

## Compatibility

Everything written by the Python version is read by this one, and vice versa:

| File | Notes |
| --- | --- |
| `config.json`, `config-<profile>.json` | 4-space indented JSON, unknown keys preserved, missing keys migrated on load |
| `credentials/<name>.json` | one file per account, atomic write via `.json.tmp` + rename |
| `credentials/accounts.txt` | line format driven by `accountFormat`, default `{email}#{password}` |
| `credentials/accounts.csv`, `accounts.jsonl` | appended when `csvExport` / `jsonlExport` are on |
| `credentials/pruned/` | backups of accounts removed by `--prune`, timestamped |
| `loop_state.json` | checkpoint used by `--resume` |
| `ENC2:` / `ENC:` / `OBS:` | encrypted credential fields |

### Encryption

Credential fields are encrypted with AES-256-GCM and written as
`ENC2:<base64(nonce || tag || ciphertext)>`. Legacy values are still readable:

- `ENC:` — Fernet (AES-128-CBC + HMAC-SHA256)
- `OBS:` — the old pycryptodome fallback

Accounts that cannot be decrypted (missing or wrong password) are reported and
skipped rather than deleted, and `--prune` never removes an encrypted account.

## Install

### Prebuilt binary

Grab the matching asset from the
[releases page](https://github.com/SchooiCodes/MegaTemp/releases).

### From source

```sh
cargo build --release
```

The binary is written to `target/release/megatem` (`megatem.exe` on Windows).
A C toolchain is needed for the TLS stack; on Windows that means the MSVC build
tools.

## Your first account

```sh
megatem
```

With no arguments the interactive menu opens. Choose *Create Accounts* and the
program will find an installed Chromium-based browser, create a throwaway
mailbox, walk MEGA's registration form, confirm the email, and save the account
to `credentials/`.

If no browser is found you are prompted for a path; it is remembered in
`config.json`.

## Common tasks

```sh
megatem -a 4 -sh                  # one account, visible browser
megatem -l 10 -j 4               # 10 accounts, 4 in parallel
megatem --keepalive --prune -y   # check every account, drop dead ones
megatem --extract                # write credentials.txt
megatem --export-bitwarden       # also: --export-onepassword, --export-keepass
megatem --list-cloud             # list files in a saved account's drive
megatem --disk-usage             # where the space went
megatem --clean                  # remove profiles, .part files, old backups
```

`--resume` picks a `--loop` run back up from `loop_state.json`.

## CLI reference

### Registration

| Flag | Meaning |
| --- | --- |
| `-a, --attempts` | attempts per provider (default 4) |
| `-l, --loop` | accounts to create sequentially |
| `-j, --parallel` | accounts to create at once |
| `--isolate` | run parallel registrations in separate processes |
| `--visible` (`-sh`) | show the browser window |
| `--keep-profiles` | keep the temporary browser profiles |
| `--provider` | `mailtm` (default) or `guerrillamail` |
| `--mail-timeout` | seconds to wait for the confirmation email |
| `--browser` | path to Chrome/Chromium/Edge |
| `--resume` | continue a previous `--loop` |

### Upload and cloud

| Flag | Meaning |
| --- | --- |
| `-f, --file` | upload this file after creating an account |
| `--upload-dir` | upload every file in a directory |
| `-p, --public` | ask MEGA for a public link |
| `--list-cloud` | list root files of the first usable account |
| `--download-cloud` | download by handle or name |
| `--download-dest` | destination directory (default `.`) |

### Export

`-e/--extract`, `--export-csv`, `--export-jsonl`, `--export-bitwarden`,
`--export-onepassword`, `--export-keepass`.

### Proxy

| Flag | Meaning |
| --- | --- |
| `--proxy` | one proxy URL |
| `--proxy-file` | file with one proxy per line |
| `--proxy-per-attempt` | rotate through the file on every attempt |
| `--proxy-url` | fetch a proxy list from a URL |

`--proxy-file` wins over `--proxy`. Parallel workers are handed proxies
round-robin, matching the Python behaviour.

### Keepalive and health

| Flag | Meaning |
| --- | --- |
| `-ka/--keepalive` | log in to every saved account and refresh `lastLogin` |
| `--workers` | keepalive concurrency (0 = automatic) |
| `--interval` | repeat keepalive every N hours |
| `--prune` | delete accounts that fail authentication |
| `--health` | print a status table |
| `--json` | machine-readable output for `--health` |

### Output, security and maintenance

`-y/--yes`, `-v/--verbose`, `--quiet`, `--profile`, `--encryption-password`,
`--webhook-url`, `--notify`, `--disk-usage`, `--clean`, `--update`, `--help`,
`--version`.

Multi-character short flags from the Python CLI (`-ka`, `-sh`, `-csv`) are
normalised before parsing.

## Interactive menu

`Accounts` → create singles or batches, `Credentials` → browse, edit notes and
tags, reveal passwords, delete, export, `Account Management` → keepalive,
storage info, delete account, create folder, `Upload & Cloud` → upload and
download, `Settings` → attempts, visible browser, exports, password generator,
config editor.

The menu falls back to numbered prompts when stdout is not a terminal, so it
also works when piped.

## Configuration

`config.json` (or `config-<profile>.json` with `--profile`) holds
`maxAttempts`, `emailProvider`, `mailTimeout`, `visibleBrowser`, `executablePath`,
`proxy`, `proxyFile`, `proxyPerAttempt`, `accountFormat`, `csvExport`,
`jsonlExport`, `encryptionPassword`, `webhookUrl`, `notify`, `quiet`.

A corrupt file is backed up and replaced with defaults instead of crashing.
CLI flags override file values for the current run only.

## Parallelism and reliability

Registrations retry with jittered backoff and fall back to the other mail
provider when one keeps failing. MEGA API calls retry `-3` responses and solve
the hashcash challenge automatically when the API returns HTTP 402. Uploads and
downloads are chunked; downloads stream to a `.part` file and are renamed only
when complete.

`--isolate` currently runs the workers as asynchronous tasks inside one
process, the same failure domain as the Python threaded mode; true process
isolation is not implemented yet.

## Disk usage

Browser profiles live under `%TEMP%\MegaTemp\<profile>` and are deleted after a
successful run (or kept with `--keep-profiles`). Profiles older than a day are
cleaned automatically at startup. `--clean` removes profiles, stray `.part`
files, update leftovers (`*.old`, `*.new`), and keeps the 100 newest pruned
backups.

## Troubleshooting

- **No browser found** — pass `--browser`, or install Chrome/Chromium/Edge.
- **Confirmation email never arrives** — try `--provider guerrillamail`, or
  raise `--mail-timeout`.
- **`MEGA API error -9` / account marked dead** — the account does not exist
  (anymore); `--prune` removes it after backing it up.
- **Credentials say "(locked)"** — `--encryption-password` is missing or wrong.
- **`link.exe not found` when building** — install the MSVC build tools, or
  build with the GNU toolchain.

## Development

```sh
cargo check
cargo test
cargo build --release
```

The test suite covers config migration, the credential store, encryption
round-trips, MEGA key/hash/attribute crypto (including byte-for-byte vectors
generated with the Python implementation), proxy rotation, password policy and
hashcash solving.

Layout:

```
src/
  cli.rs menu.rs main.rs     CLI surface, interactive menu, dispatch
  config.rs credentials.rs   config.json and the credential store
  crypto.rs lock.rs disk.rs  field encryption, file locks, cleanup
  mega/                      API client, MEGA crypto, error mapping
  mail/                      mail.tm and Guerrilla Mail
  browser/                   browser detection, CDP client, registration flow
  services/                  keepalive, upload, download, account, export
```

## Documentation

The original project's documentation applies unchanged:
<https://github.com/SchooiCodes/MegaTemp#readme>.

## License

GPL-3.0-only. This is a derivative work of
[MegaTemp](https://github.com/SchooiCodes/MegaTemp) by SchooiCodes; see
`LICENSE` for the full text.
