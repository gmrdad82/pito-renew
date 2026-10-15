---
title: Add self-update to an app
description: Wire check, install, restart and clean_old_binary into a Rust app so it updates itself from a signed manifest.
sidebar:
  order: 10
---

An app needs two things before it can update itself: the URL of its release
manifest and the minisign public key that signs it.
[Sign and publish a release](../sign-and-publish/) makes both. The crate
supplies no URL, path or host of its own; the app passes them in.

## 1. Sweep leftovers at start-up

```rust
let exe = std::env::current_exe()?;
renew::clean_old_binary(&exe);
```

`clean_old_binary` removes a leftover `<exe>.staging` (an install that was
cut off) or `<exe>.old` (the binary Windows parks during a swap). It never
fails: a missing file is fine. Call it before `install`, because `install`
refuses to start while `<exe>.staging` exists.

## 2. Check

```rust
const MANIFEST_URL: &str = "https://example.com/app/manifest.json";
const RELEASE_KEY: &str = "RWQ…";

let check = renew::check(MANIFEST_URL, RELEASE_KEY, env!("CARGO_PKG_VERSION"))?;
```

`check(url, public_key, current)` fetches the manifest (at most 1 MiB) and
its signature at the same URL with `.minisig` appended (at most 4 KiB), each
with a 3 s timeout, verifies the signature over the exact manifest bytes, and
only then reads it. It answers a `Check`:

- `update`: the newest release newer than `current`, whatever its major, with
  this platform's binary, or `None`;
- `new_major`: the same release when it is a later major than `current`, or
  `None`;
- `issued`: the manifest's `issued` time in Unix seconds, or `None`.

`check` keeps no state, so call it as often as you like: at start-up, on a
timer, or from a menu item.

### Refuse a replayed manifest

Any manifest you ever signed still verifies, so a stale cache or a host
serving an old pair can offer a release you have since superseded. When the
manifest carries `issued` ([Sign and publish](../sign-and-publish/)), call
`check_since` instead, with a floor the app keeps:

```rust
let floor = BUILT_AT.max(last_issued_seen);
let check = renew::check_since(MANIFEST_URL, RELEASE_KEY, env!("CARGO_PKG_VERSION"), floor)?;
if let Some(issued) = check.issued {
    remember_issued(issued);
}
```

`check_since(url, public_key, current, not_before)` makes the same checks and
refuses with `StaleManifest` a manifest with no `issued` or one issued before
`not_before`; one issued at exactly `not_before` passes, so the same manifest
fetched twice is fine. `BUILT_AT` is a time the release build can't predate
its manifest by, such as its commit time from the build script, and
`last_issued_seen` is the newest `issued` the app has stored.

### Refuse another app's manifest

If one key signs the manifests of several apps, any of them verifies at any
URL, so a misconfigured host or CDN could serve app B's manifest where app A
looks and A would install B. When the manifest carries `app`
([Sign and publish](../sign-and-publish/)), call `check_app`, which makes
every check `check_since` makes and also refuses with `WrongApp` a manifest
whose `app` isn't the one the app passes, or that has none:

```rust
let check = renew::check_app(MANIFEST_URL, RELEASE_KEY, "work", env!("CARGO_PKG_VERSION"), floor)?;
```

## 3. Install and relaunch

```rust
if let Some(update) = check.update {
    if check.new_major.is_some() {
        println!("{} is a new major version", update.version);
    }
    renew::install(&update.binary, &exe)?;
    renew::restart(&exe)?;
    std::process::exit(0);
}
```

`install(&binary, &exe)` downloads to `<exe>.staging`, a file it creates
itself with mode 0600, hashing as it goes, with a 180 s timeout. It checks
the exact size and the sha256, makes the file 0755, then renames it over
`exe` in one step.
On any failure it removes the staging file it made and the old binary stays
where it was.

`restart(&exe)` spawns the new binary detached, with no stdin, stdout or
stderr; the app exits after it.

### A macOS app bundle

When `exe` runs from a `.app` bundle (`<Name>.app/Contents/MacOS/<exe>`), the
same two calls update the whole bundle, because a new executable inside a
signed, notarised bundle breaks its signature. Publish the bundle zipped under
`platforms.macos-arm64.app` ([Sign and publish](../sign-and-publish/)), and
`install` downloads and checks the zip, unpacks it beside the running bundle,
refuses it unless its code signature verifies and its Team ID and identifier
match the running app's, and swaps the bundles in one step. The running
process carries on from the old bundle, kept as `<bundle>.old` until the next
`clean_old_binary`; `restart` opens the new one with `open -n`.

If the user can't write the bundle or its folder, such as an app owned by
root in `/Applications` or one run from a disk image, `install` answers
`NotWritable` before downloading anything: show that the app can't update
itself there. [Formats](../../reference/formats/#the-app-bundle-on-macos)
lists every step. This path is tested only on GitHub's macOS runners, without
a notarised bundle.

The app keeps its own ceremony around these calls (a backup first, a prompt
to the user, a quiet moment to swap) and picks when to make them.

## Handle the errors

Every failure is a `RenewError` whose message names what went wrong:

| Variant | Message |
|---|---|
| `ManifestUnreachable` | the release manifest could not be reached |
| `ManifestMalformed` | the release manifest did not parse |
| `BadPublicKey` | the public key is not a minisign public key |
| `BadSignature` | the release manifest's signature does not verify |
| `NoPlatformBinary` | no binary is published for this platform |
| `DownloadFailed` | the download did not arrive whole |
| `BadVersion` | a version string did not parse |
| `StageFailed` | the downloaded binary could not be staged: … |
| `VerifyFailed` | the downloaded binary failed verification: … |
| `InsecureUrl` | refused a plain http url: only https, or http to this machine, is fetched |
| `SwapFailed` | the new binary could not be swapped in: … |
| `NoAppBundle` | this app runs from a macOS .app bundle and no bundle is published for it |
| `NotWritable` | … is not writable by this user, so the app can't update itself |
| `StaleManifest` | the release manifest was issued before …, so it is stale or replayed |
| `WrongApp` | the release manifest is not …'s, so it was served at the wrong url |

A missing `.minisig` counts as `ManifestUnreachable`. None of them leaves a
new binary in place: an app can show the message and carry on at its current
version.

## The whole thing

```rust
use std::error::Error;

use renew::Check;

const MANIFEST_URL: &str = "https://example.com/app/manifest.json";
const RELEASE_KEY: &str = "RWQ…";

fn update() -> Result<(), Box<dyn Error>> {
    let exe = std::env::current_exe()?;
    renew::clean_old_binary(&exe);
    let Check { update, new_major, .. } =
        renew::check(MANIFEST_URL, RELEASE_KEY, env!("CARGO_PKG_VERSION"))?;
    let Some(update) = update else {
        return Ok(());
    };
    if new_major.is_some() {
        println!("{} is a new major version", update.version);
    }
    println!("installing {}", update.version);
    renew::install(&update.binary, &exe)?;
    renew::restart(&exe)?;
    std::process::exit(0);
}
```

The [API reference](../../api/renew/index.html) lists every public item,
including `platform_key()`, `staging_path()`, `AppBundle` and the timeout
and size constants.
