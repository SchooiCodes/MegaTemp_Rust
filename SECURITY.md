# Security Policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately through
[GitHub Security Advisories](https://github.com/SchooiCodes/MegaTemp_Rust/security/advisories/new)
rather than opening a public issue. Include reproduction steps, affected
versions and any mitigation you found. You should receive an acknowledgement
within a few days.

## Scope and limitations

MegaTemp is an automation tool for a third-party service. It is not affiliated
with or endorsed by MEGA. It stores credentials locally in plaintext JSON by
default; optional `ENC2:` AES-256-GCM field encryption is available through
`--encryption-password` / `config.json`. Protect the `credentials/` directory
and never commit it — it is gitignored.

The tool drives a local browser and only talks to MEGA and the configured mail
providers. Proxies and webhooks are used only when you configure them. Report
credential leaks immediately and rotate the affected accounts.

## Supported versions

Only the latest release and the `master` branch receive fixes.
