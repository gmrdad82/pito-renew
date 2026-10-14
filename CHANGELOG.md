# Changelog

## v2.0.0 — 2026-10-14

### Added

- An app that runs from a macOS `.app` bundle now updates the whole bundle, so a notarised app keeps a valid signature. The manifest's `platforms.macos-arm64.app` (`url`, `sha256`, `size`) points to a `ditto -c -k --keepParent` zip of the signed, notarised, stapled app; `install` checks the zip, unpacks it beside the running app, refuses it unless its code signature verifies against an Apple certificate of the running app's Team ID and identifier, swaps the two bundles in one step and keeps the old one until the next start. `restart` opens the new bundle.
- An app the user can't write, such as one owned by root in `/Applications` or run from a disk image, answers that it can't update itself (`NotWritable`) before downloading anything. A bundled app with no bundle in the manifest answers `NoAppBundle` instead of breaking its signature.

### Changed

- `Binary` gains `app: Option<AppBundle>` and `RenewError` gains `NoAppBundle` and `NotWritable`: code that builds a `Binary` by hand adds `app: None`, and an exhaustive `match` on `RenewError` adds the two arms. The macOS bundle path is tested on GitHub's macOS runners only, without a notarised bundle.

## v1.0.2 — 2026-10-14

### Fixed

- The docs bundle quotes each page's title, description and sidebar label, so a value containing ": " no longer breaks the docs site.

## v1.0.1 — 2026-10-14

### Added

- Guides for docs.pitomd.com: install, adding self-update to an app, signing and publishing a release, what it refuses, new major versions, measuring, and the manifest format, with the API reference from `cargo doc`.
- Each GitHub Release carries `docs-vX.Y.Z.tar.gz`, the docs bundle that `bin/docs-bundle` builds.

## v1.0.0 — 2026-10-14

### Added

- `cargo run --release --example speed` times a check and a 64 MiB update over loopback, beside `curl` then `sha256sum`.
- The demo takes `--forged` (a manifest signed by another key, refused) and `--major` (an update to a new major version).

### Changed

- pito-renew is 1.0.0. The API is the same as 0.2.1's: moving to it is only a change of tag.
- The README shows each feature with a clip or a chart, with the speed and size numbers and how they were measured.

## v0.2.1 — 2026-10-13

### Changed

- Updates cross major versions: an app at 1.4.2 now takes 2.0.0 like any other update. `new_major` holds the same release when it is a later major, so the app can say so. If a later major has no binary for this platform, the update falls back to the newest release of the running major.

## v0.2.0 — 2026-10-13

### Added

- Updates stay inside the running app's major version: 1.4.2 takes 1.5.0 and 1.5.1, patches included, and never moves to 2.0.0 by itself. A newer major is reported separately as an offer, with its binary, for the app to show as an upgrade.
- The manifest may list the newest release of each older major under `releases`, so older majors keep getting updates after a new one ships.

### Changed

- Breaking: `check` answers a `Check` with `update` and `new_major`, each a `Release` or none, in place of `Current` and `Available`.
- The brand now reads PITO MD.
- Breaking: the release manifest must be signed with minisign, and `check` takes the app's public key; an unsigned or mis-signed manifest is refused before it is read.
- Plain http is refused for the manifest and the binary, redirects included, except to this machine.
- `install` creates its staging file itself, mode 0755, and refuses when one already exists, so it never writes through a planted link or into another install's download.
- The staging and old files are named after the whole executable name, so `app` and `app.v2` in one folder no longer share them.
- Build metadata no longer makes a version newer, so `1.2.0+rebuild` doesn't reinstall over `1.2.0`.
- A sha256 that isn't 64 hex digits is refused before anything downloads.

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
