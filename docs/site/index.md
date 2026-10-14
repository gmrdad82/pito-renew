---
title: pito-renew
description: Self-update for a Rust app, in one small crate: check a signed release manifest, install this platform's binary once its sha256 matches, swap it in place and relaunch.
sidebar:
  order: 0
---

![The demo example: an app at 1.0.0 finds 1.1.0 in a signed local manifest, installs it once its sha256 matches, then refuses a tampered download and keeps the old binary](images/demo.gif)

pito-renew is self-update for a Rust app, in one small crate, `renew`, part of
[PITO MD](https://pitomd.com). It takes three calls:

- `check` fetches a signed release manifest and says whether a newer release is out;
- `install` downloads this platform's binary and swaps it in place of the running executable;
- `restart` relaunches it.

Nothing runs or stays on disk unless the manifest's minisign signature
verifies against the key the app carries and the binary's size and sha256
match what was signed.

| | |
|---|---|
| Check for an update | **0.47 ms** on loopback, signature verified |
| Download, verify and swap 64 MiB | **193 ms** on loopback, 1.2× faster than `curl` then `sha256sum` |
| Library code | **378 lines** of Rust, no `unsafe` |
| Dependencies | **7** direct, 44 in all, no system TLS library |
| Added to a stripped release binary | **1.86 MiB**, HTTPS included |

[Measure speed and size](guides/measure/) shows how each number was taken.
Works on Linux and macOS (Windows through WSL), CI-tested on both.

## Where to go

- [Install](install/): add the crate and run the demo.
- [Add self-update to an app](guides/add-self-update/): the three calls in an app.
- [Sign and publish a release](guides/sign-and-publish/): the key, the manifest, its signature.
- [What it refuses](guides/what-it-refuses/): forged manifests, tampered binaries, plain http.
- [Ship a new major version](guides/major-versions/): how updates cross majors.
- [Measure speed and size](guides/measure/): the numbers above, and how to take them again.
- [Formats](reference/formats/): the manifest, its signature and the files renew leaves beside the app.
- [API reference](api/renew/index.html): every public item, from `cargo doc`.
- [Changelog](changelog/).

The code is MIT licensed, © Catalin Ilinca. The PITO MD name and its logos
are © Catalin Ilinca, all rights reserved, and are not covered by the MIT
grant.
