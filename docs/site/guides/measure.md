---
title: Measure speed and size
description: Time a check and a 64 MiB update against curl then sha256sum, and see what renew adds to a stripped release binary.
sidebar:
  order: 14
---

renew's numbers come from one machine, measured once on 2026-10-14: AMD
Ryzen 9 9950X (16 cores), Linux 7.2, Rust 1.97.1, renew 0.2.1, whose code
is the same as 1.0's.

## Check and update speed

```sh
cargo run --release --example speed
```

It serves a signed manifest and a 64 MiB binary from a local port, times
100 `check` calls and 7 `install`s of the 64 MiB binary, and times 7 runs of

```sh
curl -fsS -o app <url> && sha256sum app && chmod 755 app
```

(curl 8.22, GNU coreutils 9.11) against the same server and file. It prints
the medians:

```text
check 0.47 ms
install 193 ms (332 MiB/s)
shell 225 ms (285 MiB/s)
```

![Paired bars: updating a 64 MiB binary takes 193 ms with renew and 225 ms with curl then sha256sum, 1.2 times faster, median of 7 on a Ryzen 9 9950X over loopback HTTP](../images/update-paired.png)

`install` hashes while it downloads, so the binary is read once; the usual
script downloads with `curl`, then reads the file back with `sha256sum`.
This is loopback HTTP, so it measures renew's own cost, not a network: over
a real network the download dominates either way, and verifying costs no
extra pass. A `check` is two small requests and one signature check, bounded
by a 3 s timeout over a real network.

## Size

![Bars: an app that prints its version is 299 KiB stripped; with renew it is 2202 KiB, HTTPS included](../images/size-bars.png)

Two copies of a small app that prints its version, one alone and one that
calls `check`, `install` and `restart`, built for linux-x86_64 with
`--release` and this profile:

```toml
[profile.release]
lto = true
codegen-units = 1
strip = true
```

They are 306,560 and 2,254,504 bytes. The difference, 1.86 MiB, is mostly
HTTPS: ureq, rustls and its crypto. A cold release build of the second took
50 s on the same machine.

## Lines and dependencies

- **378 lines** of library code: `src/` without its tests, no `unsafe`.
- **7** direct dependencies and 44 in all: `cargo tree -e normal`. HTTP is
  [ureq](https://crates.io/crates/ureq) with rustls, the signature check is
  [minisign-verify](https://crates.io/crates/minisign-verify), and the
  streaming sha256 is [pito-fetch](https://docs.pitomd.com/pito-fetch/)'s.
