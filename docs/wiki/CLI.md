# CLI guide

`megatemp` keeps every flag from the Python original. Run `megatemp --help` for
the authoritative list. Multi-character short flags (`-ka`, `-sh`, `-csv`) are
normalised automatically.

## Common commands

| Command | What it does |
| --- | --- |
| `megatemp` | Open the interactive terminal menu |
| `megatemp -a 4 -sh` | Create one account with 4 attempts, visible browser |
| `megatemp -l 10 -j 4` | Create 10 accounts, 4 at a time |
| `megatemp -l 10 -j 4 --isolate` | Same, with one process per worker |
| `megatemp -ka --prune -y` | Log in to all accounts and back up/remove dead ones |
| `megatemp --health --json` | Machine-readable health report |
| `megatemp --list-cloud` | List files in the newest account's drive |
| `megatemp -e` | Export credentials to `credentials.txt` |

## Registration

- `-a, --attempts N` — attempts per mail provider.
- `-l, --loop N` — create N accounts sequentially.
- `-j, --parallel N` — create N accounts concurrently. Combine with `-l` to
  choose both the total and the worker count; `--isolate` uses child processes.
- `--provider mailtm|guerrillamail` — temporary mailbox provider.
- `--mail-timeout SECONDS` — how long to wait for MEGA's confirmation email.
- `--browser PATH`, `--visible`, `--keep-profiles` — browser control.
- `--resume` — continue a `-l` run from `loop_state.json`.

## Upload and cloud

- `-f, --file PATH` / `--upload-dir DIR` — upload after registration.
- `-p, --public` — request a public link.
- `--list-cloud`, `--download-cloud HANDLE`, `--download-dest DIR` — manage files.
  The GUI lets you choose which saved account to browse.

## Exports

`-e/--extract`, `--export-csv`, `--export-jsonl`, `--export-bitwarden`,
`--export-onepassword`, `--export-keepass`.

## Proxies

`--proxy URL` sets one proxy; `--proxy-file FILE` loads a list;
`--proxy-per-attempt` rotates on every attempt; `--proxy-url URL` fetches a
list from a URL. An explicit `--proxy`/`--proxy-file` wins over `--proxy-url`.
Parallel workers receive proxies round-robin.

## Health and maintenance

`-ka/--keepalive` refreshes `lastLogin`; `--workers N` controls concurrency;
`--interval HOURS` repeats the check; `--prune` backs up dead accounts to
`credentials/pruned/` before deleting them. `--health` prints a dashboard and
`--json` emits machine-readable output (requires `--health`). `--disk-usage`
reports space use and `--clean` removes stale profiles, `.part` files and old
backups. `--update` self-updates from the latest release.
