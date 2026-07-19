# Security Policy

## Supported versions

Security fixes are developed for the current `release` branch and the newest
LumaDrobe preview release. Older previews are not maintained separately.

## Reporting a vulnerability

Please use GitHub's
[private vulnerability reporting](https://github.com/yuebanddd/Codex-Dream-Skin/security/advisories/new)
for vulnerabilities involving code execution, subscription integrity, local
file access, process identity, CDP isolation, or sensitive-data exposure.

If private reporting is unavailable, open an issue containing only a minimal
description and ask the maintainer for a private channel. Do not publish proof
of concept code, credentials, conversation data, `auth.json`, API keys, or
private logs in a public issue.

The maintainer will acknowledge a complete report when it is reviewed, keep the
reporter informed of material status changes, and credit the reporter if they
want attribution and coordinated disclosure is safe.

## Release trust

Official source and releases are published only from
`https://github.com/yuebanddd/Codex-Dream-Skin`. Unsigned previews must declare
`signed: false` in their build metadata. Windows releases that declare
`signed: true` must have a valid Authenticode signature and timestamp. A
checksum proves file integrity only; it does not replace a trusted signature
or guarantee acceptance by Microsoft Smart App Control.

See [SIGNING_POLICY.md](./SIGNING_POLICY.md) for the release and approval rules.
