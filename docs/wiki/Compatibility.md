# Compatibility and data formats

The Rust port reads and writes the Python original's files directly. No
migration step is needed.

| Path | Purpose |
| --- | --- |
| `config.json` / `config-<profile>.json` | 4-space JSON, unknown keys preserved, missing keys migrated on load |
| `credentials/<name>.json` | one plaintext JSON record per account |
| `credentials/accounts.txt` | line format from `accountFormat`, default `{email}#{password}` |
| `credentials/accounts.csv`, `accounts.jsonl` | appended when export is enabled |
| `credentials/pruned/` | timestamped backups of pruned accounts |
| `loop_state.json` | checkpoint for `--resume` |

A credential record has these keys: `email`, `emailPassword`, `id`, `lastLogin`,
`notes`, `password`, `tags`. Field values may be plaintext or one of:

- `ENC2:` — AES-256-GCM (`base64(nonce || tag || ciphertext)`), current default.
- `ENC:` — legacy Fernet (AES-128-CBC + HMAC-SHA256).
- `OBS:` — legacy pycryptodome fallback.

Set `encryptionPassword` in `config.json` or pass `--encryption-password` to
read encrypted values. Locked accounts are skipped, never silently deleted;
`--prune` also refuses to remove them.

## MEGA internals

Login mirrors `mega.py`: PBKDF2-SHA512(password, salt, 100000, 32), AES key from
the first 16 bytes and `uh` from the last 16. AES-CBC selects 128/192/256-bit
keys by length. File node attributes and download CTR use the key reconstruction
`k = (k0^k4, k1^k5, k2^k6, k3^k7)`.

## Browser profiles

Temporary Chromium profiles live under `%TEMP%\MegaTemp` and are deleted after a
successful run unless `--keep-profiles` is used. `--clean` removes stale
profiles, partial downloads and old backups.
