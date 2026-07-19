# LumaDrobe Privacy Policy

Last updated: 2026-07-19

LumaDrobe is an open-source desktop client. It has no telemetry, advertising,
analytics, crash-reporting service, user account, or LumaDrobe-operated server.

This program will not transfer any information to other networked systems
unless specifically requested by the user or the person installing or
operating it.

## Network access

LumaDrobe makes network requests only when the user asks it to add or refresh a
theme subscription, or to download and install a theme from that subscription.
Those requests go to the public Git repository and resource URLs selected by
the user. GitHub-hosted subscriptions are subject to the
[GitHub Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).
Other subscription hosts are governed by their own policies.

The client does not send Codex conversations, prompts, form values, API keys,
authentication files, provider settings, or browser storage to a LumaDrobe
service. LumaDrobe does not operate a service that could receive them.

## Local data

The client stores subscription metadata, downloaded themes, integrity hashes,
installation state, runtime state, preferences, and rotating diagnostic logs in
its operating-system application-data directory. Theme resources remain local
after installation until the user removes them.

The runtime log records product/build versions, selected theme identifiers,
local paths, process and loopback CDP identity, ports, failure stages, and
bounded structural diagnostics. It does not intentionally record page text,
conversation content, titles, form values, URL query strings, browser storage,
theme CSS or images, API keys, `auth.json`, or Codex provider configuration.

## Local Codex connection

When the user applies a theme, LumaDrobe connects only to a verified Codex
desktop process over a loopback CDP endpoint. It does not expose the endpoint to
the network, modify the official Codex package, or upload CDP data.

## User control and disclosure

Removing a subscription, deleting an installed theme, or uninstalling the
client removes the corresponding LumaDrobe-managed data according to the
operating system's normal behavior. Diagnostic logs are shared only when the
user explicitly exports or uploads them. Logs can contain local paths and
process identifiers, so users should review them before public disclosure.

Questions or corrections can be submitted through the repository's
[issue tracker](https://github.com/yuebanddd/Codex-Dream-Skin/issues).
