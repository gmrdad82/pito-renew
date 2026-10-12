<p align="center"><img src="docs/images/demo.gif" alt="The demo example: an app at 1.0.0 finds 1.1.0 in a local manifest, installs it once its sha256 matches, then refuses a tampered download and keeps the old binary"></p>

# pito-renew

[![CI](https://github.com/gmrdad82/pito-renew/actions/workflows/ci.yml/badge.svg)](https://github.com/gmrdad82/pito-renew/actions/workflows/ci.yml)
[![Version](https://img.shields.io/github/v/tag/gmrdad82/pito-renew)](https://github.com/gmrdad82/pito-renew/tags)

Self-update for a Rust app, as a small crate, `renew`. It fetches a release
manifest, picks this platform's binary, streams it to disk under a size cap
while hashing it, refuses it unless its size and sha256 match the manifest,
swaps it in place of the running executable and relaunches it. The app keeps
its own ceremony around the update, such as a backup first, and picks its own
moment to restart. It is the self-update of the [PITO](https://pitomd.com)
apps.

Works on Linux and macOS (Windows through WSL), CI-tested on both. The crate
also builds for native Windows, where the swap parks the running binary as
`<exe>.old` first; CI doesn't run that path.

## Install

It isn't on crates.io; add it from git, pinned to a release tag:

```toml
renew = { git = "https://github.com/gmrdad82/pito-renew", tag = "v0.1.2" }
```

It uses [ureq](https://crates.io/crates/ureq) with rustls for HTTP, so it
needs no system TLS library.

## Use

Serve a manifest over HTTPS from a host you control:

```json
{
  "version": "1.1.0",
  "platforms": {
    "linux-x86_64": { "url": "https://example.com/app/1.1.0/app-linux-x86_64", "sha256": "…", "size": 10485760 },
    "macos-arm64": { "url": "https://example.com/app/1.1.0/app-macos-arm64", "sha256": "…", "size": 9437184 }
  }
}
```

The platform keys are `linux-x86_64`, `macos-arm64` and `windows-x86_64`;
`renew::platform_key()` gives this build's key, or an empty string on any
other platform. Each `url` points to the raw executable, not an archive.

Then, in the app:

```rust
use std::error::Error;

use renew::Check;

fn update() -> Result<(), Box<dyn Error>> {
    let exe = std::env::current_exe()?;
    renew::clean_old_binary(&exe);
    match renew::check("https://example.com/app/manifest.json", env!("CARGO_PKG_VERSION"))? {
        Check::Current => Ok(()),
        Check::Available { version, binary } => {
            println!("installing {version}");
            renew::install(&binary, &exe)?;
            renew::restart(&exe)?;
            std::process::exit(0);
        }
    }
}
```

- `check(url, current)` fetches the manifest (3 s timeout, at most 1 MiB)
  and compares its version with the running one by semver. It answers
  `Check::Current`, or `Check::Available` with the new version and this
  platform's binary, and refuses a published size of 0 or over 512 MiB.
- `install(&binary, &exe)` downloads to `<exe>.staging` (180 s timeout),
  stops as soon as more bytes
  arrive than were published, checks the exact size and the sha256, makes the
  file executable and renames it over `exe`. On any failure the staging file
  is removed and the old binary stays where it was.
- `restart(&exe)` spawns the new binary detached, with no stdin, stdout or
  stderr; the app exits after it.
- `clean_old_binary(&exe)` sweeps a leftover `<exe>.staging` or `<exe>.old`;
  call it at start-up.

Every failure is a `RenewError` naming what went wrong: the manifest
unreachable or malformed, no binary for this platform, a failed download, a
bad version string, or a staging, verification or swap error. `renew` trusts
the manifest it is given, so serve it over HTTPS.

## Try it

```sh
cargo run --example demo
cargo run --example demo -- --tampered
```

The demo is the clip above. It serves a manifest and a binary from a local
port, then updates an app at 1.0.0 in a temporary folder to 1.1.0. With
`--tampered` the server sends other bytes of the same size: the checksum
doesn't match, the install is refused and the old binary stays. Its recording
is kept in `render/`.

## Build and test

```sh
bin/gate
```

runs rustfmt, clippy with warnings as errors, and the tests. The tests serve
their downloads from a local port and write their scratch files under
`tmp/`, so they need no network.

## Contributing

Issues and pull requests are welcome. Please read the
[code of conduct](CODE_OF_CONDUCT.md) first. A change keeps `bin/gate` green
with no warnings, and never lets a binary run or stay in place unless its
size and sha256 match the manifest. Report a security issue privately, as
[SECURITY.md](SECURITY.md) says, not in a public issue.

## Licence

The code is MIT licensed, © Catalin Ilinca: see [LICENSE](LICENSE). The MIT
grant covers the code only: the PITO name and its logos are © Catalin Ilinca,
all rights reserved, and are not covered by it; see
[TRADEMARKS.md](TRADEMARKS.md).
