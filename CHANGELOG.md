# Changelog

## Unreleased

### Changed

- The brand now reads PITO MD.

## v0.1.2 — 2026-10-12

### Added

- A demo example that updates an app in a temporary folder from a local manifest, and refuses a tampered download with `--tampered`; its clip opens the README.
- The MIT licence, a code of conduct, a security policy and CI on GitHub for Linux and macOS.

### Changed

- Fetch v0.1.2, from its public repository over https.
- The tests write their scratch files under the crate's own `tmp/`.

## v0.1.1 — 2026-09-28

### Changed

- Fetch v0.1.1, which polls body reads on ureq 3.4.1 and later.

## v0.1.0 — 2026-09-28

### Added

- The `renew` crate: check a release manifest, install this platform's binary once its size and sha256 match, swap it in place and relaunch.
