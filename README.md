<p align="center"><img src="docs/images/demo.gif" alt="The demo example: an app at 1.0.0 finds 1.1.0 in a local manifest, installs it once its sha256 matches, then refuses a tampered download and keeps the old binary"></p>

# pito-renew

[![CI](https://github.com/gmrdad82/pito-renew/actions/workflows/ci.yml/badge.svg)](https://github.com/gmrdad82/pito-renew/actions/workflows/ci.yml)
[![Version](https://img.shields.io/github/v/tag/gmrdad82/pito-renew)](https://github.com/gmrdad82/pito-renew/tags)

Self-update for a Rust app, as a small crate, `renew`. It fetches a release
manifest, refuses it unless its minisign signature verifies against the app's
public key, picks this platform's binary, streams it to disk under a size cap
while hashing it, refuses it unless its size and sha256 match the manifest,
swaps it in place of the running executable and relaunches it. The app keeps
its own ceremony around the update, such as a backup first, and picks its own
moment to restart. It is the self-update of the [PITO MD](https://pitomd.com)
apps.

Works on Linux and macOS (Windows through WSL), CI-tested on both. The crate
also builds for native Windows, where the swap parks the running binary as
`<exe>.old` first; CI doesn't run that path.

## Install

It isn't on crates.io; add it from git, pinned to a release tag:

```toml
renew = { git = "https://github.com/gmrdad82/pito-renew", tag = "v0.2.1" }
```

It uses [ureq](https://crates.io/crates/ureq) with rustls for HTTP, so it
needs no system TLS library, and
[minisign-verify](https://crates.io/crates/minisign-verify) to check the
manifest's signature.

## Use

Write a manifest:

```json
{
  "version": "1.1.0",
  "platforms": {
    "linux-x86_64": { "url": "https://example.com/app/1.1.0/app-linux-x86_64", "sha256": "…", "size": 10485760 },
    "macos-arm64": { "url": "https://example.com/app/1.1.0/app-macos-arm64", "sha256": "…", "size": 9437184 }
  }
}
```

The top-level `version` and `platforms` are the newest release. Updates
cross major versions: an app at 1.4.2 takes 2.0.0 like any other update. If
a new major ships without a binary for some platform, list the newest release
of each older major under `releases`, so apps on that platform keep getting
that major's updates:

```json
{
  "version": "2.0.0",
  "platforms": { "linux-x86_64": { "url": "…", "sha256": "…", "size": 11534336 } },
  "releases": [
    { "version": "1.5.1", "platforms": { "linux-x86_64": { "url": "…", "sha256": "…", "size": 10485760 } } }
  ]
}
```

The platform keys are `linux-x86_64`, `macos-arm64` and `windows-x86_64`;
`renew::platform_key()` gives this build's key, or an empty string on any
other platform. Each `url` points to the raw executable, not an archive, and
each `sha256` is its 64 hex digits.

Sign it with [minisign](https://jedisct1.github.io/minisign/). Make a key pair
once with `minisign -G`, and keep the secret key off the server that hosts
the releases. For each release, `minisign -Sm manifest.json` writes
`manifest.json.minisig` next to the manifest; serve both over HTTPS, the
signature at the manifest's URL with `.minisig` appended. The app carries the
public key, the second line of `minisign.pub` (it starts with `RW`).

Then, in the app:

```rust
use std::error::Error;

use renew::Check;

const RELEASE_KEY: &str = "RWQ…";

fn update() -> Result<(), Box<dyn Error>> {
    let exe = std::env::current_exe()?;
    renew::clean_old_binary(&exe);
    let Check { update, new_major } = renew::check(
        "https://example.com/app/manifest.json",
        RELEASE_KEY,
        env!("CARGO_PKG_VERSION"),
    )?;
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

- `check(url, public_key, current)` fetches the manifest (at most 1 MiB) and
  its `.minisig` (at most 4 KiB), each with a 3 s timeout, and verifies the
  signature over the exact manifest bytes before it reads them. It refuses an
  unsigned or mis-signed manifest, plain http to any host but this machine,
  a sha256 that isn't 64 hex digits, and a published size of 0 or over
  512 MiB. It compares every signed version with the running one by semver,
  ignoring build metadata, and answers a `Check` with two fields, each a
  `Release` (the version and this platform's binary) or `None`: `update`, the
  newest release newer than the running one, whatever its major, and
  `new_major`, the same release when it is a later major than the running
  one, so the app can tell the user. A later major with no binary for this
  platform is skipped, and `update` falls back to the newest release of the
  running major. An
  older or equal signed manifest replayed by the host has no `update`. It
  keeps no state, so the app calls it on its own schedule and whenever the
  user asks to check for updates.
- `install(&binary, &exe)` downloads over HTTPS (plain http only to this
  machine) to `<exe>.staging` (180 s timeout), which it creates itself, mode
  0755, and never through an existing file or link: when `<exe>.staging`
  already exists, another install is in flight or one was cut short, and it
  refuses. It stops as soon as more bytes arrive than were published, checks
  the exact size and the sha256, and renames the file over `exe`. On any
  failure the staging file it made is removed and the old binary stays where
  it was.
- `restart(&exe)` spawns the new binary detached, with no stdin, stdout or
  stderr; the app exits after it.
- `clean_old_binary(&exe)` sweeps a leftover `<exe>.staging` or `<exe>.old`;
  call it at start-up.

Every failure is a `RenewError` naming what went wrong: the manifest
unreachable (its signature missing included) or malformed, a public key that
isn't a minisign key, a signature that doesn't verify, a plain http URL, no
binary for this platform, a failed download, a bad version string, or a
staging, verification or swap error.

## Try it

```sh
cargo run --example demo
cargo run --example demo -- --tampered
```

The demo is the clip above. It makes a key pair, signs a manifest, serves it
with its signature and a binary from a local port, then updates an app at
1.0.0 in a temporary folder to 1.1.0. With
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
with no warnings, and never lets a binary run or stay in place unless the
manifest's signature verifies and the binary's size and sha256 match it.
Report a security issue privately, as [SECURITY.md](SECURITY.md) says, not in
a public issue.

## Licence

The code is MIT licensed, © Catalin Ilinca: see [LICENSE](LICENSE). The MIT
grant covers the code only: the PITO MD name and its logos are © Catalin Ilinca,
all rights reserved, and are not covered by it; see
[TRADEMARKS.md](TRADEMARKS.md).
