# FAQ

## Is this affiliated with MEGA?

No. MegaTemp is an independent, GPL-3.0-licensed automation tool. Bulk account
creation may violate MEGA's terms of service; use it only where permitted.

## Where are my accounts stored?

In `credentials/` next to the binary's working directory. By default fields are
plaintext JSON. Enable `encryptionPassword` in `config.json` (or pass
`--encryption-password`) to store `ENC2:` values. Never commit this directory.

## Why does the GUI need a browser?

Registration is automated through the Chrome DevTools Protocol against an
installed Chromium-based browser. Settings can download Chrome for Testing if
none is installed. The account-creation step cannot run without a browser.

## Why do accounts show as `(locked)`?

The credential field is encrypted with a different or missing password. Provide
the correct `--encryption-password`; locked accounts are skipped and are never
pruned.

## Why did a confirmation email never arrive?

Temporary mail providers rate-limit. The tool retries and falls back to the
other provider. Try `--provider guerrillamail`, raise `--mail-timeout`, or use a
different network. Guerrilla Mail cannot render MEGA's HTML email reliably, so
mail.tm is the default.

## What does `--prune` do?

It logs in to every account; accounts that fail authentication are backed up to
`credentials/pruned/` and then removed. Encrypted/locked accounts are left alone.

## Why do parallel workers have `[wN]` tags?

Each worker's output is buffered and printed as one block so the log stays
readable. `--isolate` goes further and runs each worker in a separate process.

## Where do I report a bug?

Open an issue using the bug report template. For security issues, use
[private security advisories](https://github.com/SchooiCodes/MegaTemp_Rust/security/advisories/new)
instead. Remove emails and passwords from any pasted logs.
