---
title: Install
description: Add renew to a Rust app on Linux or macOS, and run the demo that updates an app from a local signed manifest.
sidebar:
  order: 1
---

renew isn't on crates.io. Add it from git, pinned to a release tag, in the
app's `Cargo.toml`:

```toml
[dependencies]
renew = { git = "https://github.com/gmrdad82/pito-renew", tag = "@VERSION@" }
```

The same line works on every platform renew supports:

- **Linux x86_64**: tested on Linux.
- **macOS on Apple Silicon**: tested on macOS.
- **Windows**: through WSL, where it is Linux. The crate also builds for
  native Windows, where the swap parks the running binary as `<exe>.old`
  first; the tests don't run that path, so it is untested.

It needs Rust 1.97 or later and no system library: HTTPS is
[ureq](https://crates.io/crates/ureq) with rustls.

## Quick start

See it work with no network and nothing to set up, from a checkout:

```sh
git clone https://github.com/gmrdad82/pito-renew && cd pito-renew
cargo run --example demo
```

The demo makes a key pair, signs a manifest, serves the manifest, its
signature and a binary from a local port, and updates an app at 1.0.0 in a
temporary folder to 1.1.0:

```text
running   app 1.0.0 on linux-x86_64
check     1.1.0 is out, 10 bytes
install   ✓ the sha256 matches, swapped in place
on disk   app 1.1.0
leftover  none
```

`--tampered`, `--forged` and `--major` show the refusals and a new major:
see [What it refuses](../guides/what-it-refuses/) and
[Ship a new major version](../guides/major-versions/).

## In an app

At start-up, or when the user asks to check for updates:

```rust
const MANIFEST_URL: &str = "https://example.com/app/manifest.json";
const RELEASE_KEY: &str = "RWQ…";

fn update() -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    renew::clean_old_binary(&exe);
    let check = renew::check(MANIFEST_URL, RELEASE_KEY, env!("CARGO_PKG_VERSION"))?;
    if let Some(update) = check.update {
        renew::install(&update.binary, &exe)?;
        renew::restart(&exe)?;
        std::process::exit(0);
    }
    Ok(())
}
```

[Add self-update to an app](../guides/add-self-update/) walks through each
call, and [Sign and publish a release](../guides/sign-and-publish/) makes the
manifest and the key it checks against.
