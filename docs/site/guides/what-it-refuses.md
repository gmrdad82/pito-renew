---
title: What it refuses
description: Run the demo's forged and tampered cases, and see each check renew makes before a binary runs or stays on disk.
sidebar:
  order: 12
---

renew lets nothing run or stay on disk unless the manifest's signature
verifies and the binary matches what was signed. The demo shows both
refusals.

## A manifest it didn't sign

```sh
cargo run --example demo -- --forged
```

![demo --forged: the manifest is signed by a key the app doesn't carry; check answers that the release manifest's signature does not verify, and the app stays at 1.0.0](../images/forged.gif)

```text
running   app 1.0.0 on linux-x86_64
check     ✗ the release manifest's signature does not verify
on disk   app 1.0.0
```

The manifest is signed by a key the app doesn't carry. `check` verifies the
signature over the exact bytes before it reads a single field, so a hijacked
host or a swapped manifest can't point the app anywhere.

## A binary that doesn't match

```sh
cargo run --example demo -- --tampered
```

The second half of the clip at the top of the [overview](../../): the server
hands out other bytes of the same size.

```text
running   app 1.0.0 on linux-x86_64
check     1.1.0 is out, 10 bytes
install   ✗ the downloaded binary failed verification: the checksum does not match the published one
on disk   app 1.0.0
leftover  none
```

`install` stops as soon as more bytes arrive than were published, checks the
exact size and the sha256, and on any mismatch removes its staging file and
leaves the old binary where it was.

## Every check

Before reading the manifest, `check` refuses:

- a URL with plain http to any host but this machine, redirects included;
- a manifest over 1 MiB or a signature over 4 KiB;
- a public key that isn't a minisign key;
- a missing signature, or one that doesn't verify over the exact bytes.

Before downloading, it refuses:

- a sha256 that isn't 64 hex digits;
- a published size of 0, or over 512 MiB (1 GiB for a macOS `app` zip);
- a manifest with no binary for this platform.

With `check_app`, it also refuses a manifest issued before the app's floor,
or whose `app` isn't this app's.

While installing, `install` refuses:

- a sha256 that isn't 64 hex digits, or a size of 0 or over the caps, even
  for a `Binary` the app built or stored itself;
- plain http to any host but this machine, redirects included;
- to start when `<exe>.staging` already exists, so it never writes through a
  planted file or link, or into another install's download;
- more bytes than were published, fewer, or a sha256 that differs.

The staging file is 0600 while it downloads and becomes 0755 only after its
size and sha256 match, so unverified bytes are never executable.

For an app that runs from a macOS `.app` bundle, `install` also refuses:

- an entry with no `app` bundle, rather than put a new executable inside a
  signed bundle;
- a bundle or folder this user can't write, before downloading anything;
- a running app with no Team ID to match;
- a zip that doesn't hold exactly one `.app` folder;
- a new bundle whose code signature doesn't verify
  (`codesign --verify --deep --strict`), or whose Team ID or identifier
  differs from the running app's.

And versions compare by semver, ignoring build metadata, so `1.2.0+rebuild`
doesn't reinstall over `1.2.0`, and a replayed older manifest offers no
update.
