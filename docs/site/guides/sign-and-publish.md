---
title: Sign and publish a release
description: Make a minisign key pair, write the release manifest with each platform's sha256 and size, sign it and serve both files over HTTPS.
sidebar:
  order: 11
---

renew reads a release manifest only once its minisign signature verifies
against the public key the app carries. Publishing a release is four steps.

## 1. Make a key pair, once

```sh
minisign -G
```

This writes `minisign.pub` and a secret key (by default under
`~/.minisign/`). Keep the secret key off the server that hosts the releases.
The app carries the public key: the second line of `minisign.pub`, which
starts with `RW`.

```rust
const RELEASE_KEY: &str = "RWQ…";
```

## 2. Put each binary where the app can fetch it

Each platform gets the raw executable, not an archive, at an HTTPS URL. Take
its sha256 and its size in bytes:

```sh
sha256sum app-linux-x86_64          # Linux
stat -c %s app-linux-x86_64

shasum -a 256 app-macos-arm64       # macOS
stat -f %z app-macos-arm64
```

An app that runs from a macOS `.app` bundle also publishes the bundle, after
it is signed, notarised and stapled, zipped with `ditto` so its signature,
links and extended attributes survive:

```sh
ditto -c -k --keepParent App.app App-macos-arm64.app.zip
shasum -a 256 App-macos-arm64.app.zip
stat -f %z App-macos-arm64.app.zip
```

## 3. Write the manifest

```json
{
  "version": "1.1.0",
  "platforms": {
    "linux-x86_64": { "url": "https://example.com/app/1.1.0/app-linux-x86_64", "sha256": "…", "size": 10485760 },
    "macos-arm64": {
      "url": "https://example.com/app/1.1.0/app-macos-arm64", "sha256": "…", "size": 9437184,
      "app": { "url": "https://example.com/app/1.1.0/App-macos-arm64.app.zip", "sha256": "…", "size": 12582912 }
    }
  }
}
```

The platform keys are `linux-x86_64`, `macos-arm64` and `windows-x86_64`;
`renew::platform_key()` gives the running build's key. Each `sha256` is 64
hex digits and each `size` is above 0 and at most 512 MiB, or 1 GiB for an
`app` zip.

Add `"issued"`, the time you sign it in Unix seconds (`date +%s`), at the top
level. It sits inside the signed bytes, so an app that calls `check_since`
can refuse an older signed manifest served again.
[Formats](../../reference/formats/) has every field and limit.

## 4. Sign it and serve both files

```sh
minisign -Sm manifest.json
```

This writes `manifest.json.minisig` next to the manifest. Serve both over
HTTPS, the signature at the manifest's URL with `.minisig` appended:

```text
https://example.com/app/manifest.json
https://example.com/app/manifest.json.minisig
```

Sign again every time the manifest changes: renew verifies the signature over
the exact bytes it fetched, so a manifest edited after signing is refused,
even a change of whitespace.

## Try it locally

`cargo run --example demo` in a checkout does all four steps in a temporary
folder, with a key it makes on the spot, and serves the files from a local
port; plain http is allowed only to this machine. Its source,
`examples/demo.rs`, is a working publisher and client in one file.
