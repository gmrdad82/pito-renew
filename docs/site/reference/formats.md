---
title: Formats
description: The release manifest renew reads, its minisign signature, the platform keys, and the files renew leaves beside the app's executable.
sidebar:
  order: 20
---

## The release manifest

A JSON file, at most 1 MiB, served over HTTPS (plain http only to this
machine). The top level is the newest release:

```json
{
  "version": "1.1.0",
  "platforms": {
    "linux-x86_64": { "url": "https://example.com/app/1.1.0/app-linux-x86_64", "sha256": "…", "size": 10485760 },
    "macos-arm64": { "url": "https://example.com/app/1.1.0/app-macos-arm64", "sha256": "…", "size": 9437184 }
  },
  "releases": [
    { "version": "1.0.4", "platforms": { "windows-x86_64": { "url": "…", "sha256": "…", "size": 8388608 } } }
  ]
}
```

| Field | Type | Meaning |
|---|---|---|
| `version` | string | A semver version. Build metadata is ignored when comparing. |
| `platforms` | object | One entry per platform key, below. Other keys are ignored. |
| `platforms.<key>.url` | string | The raw executable, not an archive; https, or http to this machine. |
| `platforms.<key>.sha256` | string | The executable's sha256: exactly 64 hex digits. |
| `platforms.<key>.size` | integer | The executable's size in bytes: above 0, at most 512 MiB (536870912). |
| `releases` | array, optional | More releases with the same `version` and `platforms`, typically the newest of each older major. |

renew picks among the top-level release and every entry of `releases`, by
version: the newest release of a later major that has this platform's binary
if there is one, otherwise the newest newer release of the running major. See
[Ship a new major version](../../guides/major-versions/).

## Platform keys

| Key | Build |
|---|---|
| `linux-x86_64` | Linux on x86_64 |
| `macos-arm64` | macOS on Apple Silicon |
| `windows-x86_64` | Windows on x86_64 |

`renew::platform_key()` gives the running build's key, or an empty string on
any other platform.

## The signature

A minisign signature of the manifest's exact bytes, as `minisign -Sm
manifest.json` writes it, at most 4 KiB, served at the manifest's URL with
`.minisig` appended. The app passes the public key to `check` as text: the
second line of `minisign.pub`, starting with `RW`.

## Files beside the executable

| File | Written by | Left behind |
|---|---|---|
| `<exe>.staging` | `install`, created new with mode 0755 | Only if the process dies mid-install; removed on every failure `install` sees. |
| `<exe>.old` | `install` on Windows, which parks the running binary there before the swap | Until the next `clean_old_binary`. |

`<exe>` is the whole executable path, so `app` and `app.v2` in one folder
never share these files. `clean_old_binary(&exe)` removes both; `install`
refuses to start while `<exe>.staging` exists.

## Timeouts and limits

| Constant | Value |
|---|---|
| `MANIFEST_TIMEOUT` | 3 s, for the manifest and for its signature |
| `DOWNLOAD_TIMEOUT` | 180 s, for the binary |
| `MAX_BINARY_BYTES` | 512 MiB |
