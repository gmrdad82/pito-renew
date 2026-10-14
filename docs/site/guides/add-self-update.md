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
  `None`.

`check` keeps no state, so call it as often as you like: at start-up, on a
timer, or from a menu item.

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
itself with mode 0755, hashing as it goes, with a 180 s timeout. It checks
the exact size and the sha256, then renames the file over `exe` in one step.
On any failure it removes the staging file it made and the old binary stays
where it was.

`restart(&exe)` spawns the new binary detached, with no stdin, stdout or
stderr; the app exits after it.

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
    let Check { update, new_major } =
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
including `platform_key()`, `staging_path()` and the timeout constants.
