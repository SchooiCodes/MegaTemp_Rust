# GUI guide

`megatemp-gui` is a native egui/eframe app that uses the same library and
`config.json` as the CLI. It starts maximized; every screen scrolls, and the
top-bar button toggles maximize/restore.

## Screens

- **Overview** — searchable account table with counts, quick refresh and a
  shortcut to account details. Click an email to open it.
- **Create** — batch size, worker count, provider, attempts and browser
  visibility. Shows per-account results and a live activity log.
- **Keep alive** — check every account, optionally prune dead ones, and filter
  the results. Quota and authentication errors are shown per account.
- **Cloud** — choose a saved account, list its files, filter/sort them, upload
  a local file, download by handle or copy the handle.
- **Accounts** — reveal/copy credentials, edit tags and notes, create folders,
  delete accounts, and start a keepalive run.
- **Settings** — registration defaults, exports, network/proxy settings and
  security options. Save writes `config.json`, shared with the CLI.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+1` … `Ctrl+6` | Switch between the six screens |
| `Ctrl+N` | Open Create |
| `Ctrl+R` | Refresh the current screen |
| `Esc` | Return to Overview |

The `?` button in the top bar opens a quick guide inside the app. Notifications
appear as toasts at the bottom of the window.

## Data and privacy

The GUI writes to the same `credentials/`, `config.json` and `loop_state.json`
as the CLI. It does not phone home; only MEGA, the selected mail provider and
any webhook/proxy you configure are contacted.
