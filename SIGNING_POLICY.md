# LumaDrobe Code Signing Policy

Free code signing provided by [SignPath.io](https://signpath.io/), certificate
by [SignPath Foundation](https://signpath.org/).

## Scope

Only the LumaDrobe desktop client built from this repository may be signed. The
policy excludes Codex, third-party applications, subscribed themes, legacy
platform scripts, documentation images, and any binary that cannot be traced to
the exact public source commit and GitHub Actions workflow that produced it.

When SignPath mode is enabled, the Windows application executable is signed
before bundling and the resulting NSIS installer is signed in a second request.
Both signatures must be valid, chain to a trusted root, identify
`SignPath Foundation`, and contain a trusted timestamp before a signed Windows
asset can reach a GitHub Release.

## Trusted build and release origin

- Repository: <https://github.com/yuebanddd/Codex-Dream-Skin>
- Release branch: `release`
- Build system: GitHub Actions on GitHub-hosted runners
- Release workflow: `.github/workflows/preview-packages.yml`
- Artifact configuration: `.signpath/artifact-configuration.xml`
- Dependency/build caches: disabled for release package jobs
- Source identity: the exact immutable `release` push commit
- Publication: GitHub Pre-release attached to that same commit

Pull requests build unsigned test artifacts so untrusted changes never receive
signing credentials. Until Foundation approval is complete, release pushes may
publish explicitly labelled unsigned previews when `SIGNPATH_ENABLED` is not
`true`. Once the owner enables SignPath mode, there is no unsigned fallback:
missing configuration, a denied request, a signature mismatch, a missing
timestamp, or an asset/hash mismatch fails the workflow and prevents release.

## Project roles

- Authors: [yuebanddd](https://github.com/yuebanddd)
- Committers and reviewers: [yuebanddd](https://github.com/yuebanddd)
- Approvers: [yuebanddd](https://github.com/yuebanddd)

Changes to build workflows, dependencies, theme validation, filesystem access,
process discovery, CDP isolation, or this policy require explicit review before
merge. Signing approval is limited to commits already reviewed and merged into
`release`. The signing workflow, release metadata checks, and policy documents
have an explicit owner in `.github/CODEOWNERS`.

All role holders must use multi-factor authentication for GitHub and SignPath.
Foundation-sponsored releases require manual approval of every signing request;
the release workflow does not auto-approve requests.

## User verification

Users should inspect `BUILD-INFO.json` first. Unsigned previews declare
`signed: false` and provide integrity metadata without claiming publisher
identity. Signed releases declare `signed: true`; users should then verify the
Authenticode status, publisher, timestamp, release tag, build commit, and
SHA-256 metadata. The expected public publisher for a Foundation-sponsored
signature is `SignPath Foundation`.

## Privacy

The signed application follows [PRIVACY.md](./PRIVACY.md). It does not include
telemetry or transfer information to a LumaDrobe-operated service.
