---
title: Formats
description: The release manifest renew reads, its minisign signature, the platform keys, the macOS app bundle, and the files renew leaves beside the app.
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
    "macos-arm64": {
      "url": "https://example.com/app/1.1.0/app-macos-arm64", "sha256": "…", "size": 9437184,
      "app": { "url": "https://example.com/app/1.1.0/App-macos-arm64.app.zip", "sha256": "…", "size": 12582912 }
    }
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
| `platforms.<key>.app` | object, optional | macOS only: the whole signed `.app`, for an app that runs from a bundle. Below. |
| `releases` | array, optional | More releases with the same `version` and `platforms`, typically the newest of each older major. |
| `issued` | integer, optional | When the manifest was signed, in Unix seconds. `check_since` and `check_app` refuse a manifest without it or issued before the floor the app passes. |
| `app` | string, optional | The app the manifest is for. `check_app` refuses a manifest without it or naming another app. |

renew picks among the top-level release and every entry of `releases`, by
version: the newest release of a later major that has this platform's binary
if there is one, otherwise the newest newer release of the running major. See
[Ship a new major version](../../guides/major-versions/).

## The app bundle on macOS

An app that runs from a `.app` bundle (its executable at
`<Name>.app/Contents/MacOS/`) can't take a new executable alone: the
executable's code signature seals the bundle's `Info.plist` and
`_CodeSignature`, so a new binary inside the old bundle fails
`codesign --verify` and Gatekeeper may call the app damaged. For such an app
the `macos-arm64` entry carries `app`, the whole bundle:

| Field | Type | Meaning |
|---|---|---|
| `app.url` | string | A zip of the signed, notarised, stapled `.app`, made with `ditto -c -k --keepParent <Name>.app <zip>`; https, or http to this machine. |
| `app.sha256` | string | The zip's sha256: exactly 64 hex digits. |
| `app.size` | integer | The zip's size in bytes: above 0, at most 1 GiB (1073741824). |

The binary's own `url`, `sha256` and `size` stay required beside it, for a
copy that runs outside a bundle. When the executable `install` is given lives
in a bundle, `install`:

1. refuses with `NoAppBundle` if the entry has no `app`, rather than break the
   bundle's signature;
2. refuses with `NotWritable` if this user can't write the bundle or the
   folder holding it (an app owned by root in `/Applications`, a read-only
   disk image, a translocated app), before downloading anything;
3. reads the running bundle's Team ID and identifier with `codesign`, and
   refuses one with no Team ID;
4. downloads the zip to `<bundle>.staging/`, checking size and sha256 as for a
   binary, and unpacks it there with `ditto -x -k`; the zip must hold exactly
   one `.app` folder;
5. refuses the new bundle unless its Team ID and identifier equal the running
   bundle's and `codesign --verify --deep --strict` passes against the
   requirement `anchor apple generic and identifier "<id>" and certificate
   leaf[subject.OU] = "<team>"`, so only a certificate Apple issued to that
   team can sign it;
6. clears its quarantine flag, touches it so Launch Services sees it, and
   asks `gktool scan` to warm Gatekeeper where macOS has it (14.4 and later);
7. swaps the two bundles in one step with `renamex_np(RENAME_SWAP)`, or with
   two renames when the volume can't swap, putting the old one back if the
   second fails;
8. moves the old bundle, which the running process still uses, to
   `<bundle>.old`.

`restart(&exe)` then opens the new bundle with `open -n`, and the next
`clean_old_binary(&exe)` removes `<bundle>.old`. The swap follows Sparkle's
(see INSPIRATION.md). Tested only on GitHub's macOS runners, with bundles
that aren't notarised: the refusals and the swap are exercised there, a
notarised end-to-end update is not.

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
| `<exe>.staging` | `install`, created new with mode 0600, made 0755 once it verifies | Only if the process dies mid-install; removed on every failure `install` sees. |
| `<exe>.old` | `install` on Windows, which parks the running binary there before the swap | Until the next `clean_old_binary`. |

| `<bundle>.staging/` | `install` for an app in a macOS bundle: the zip, then the unpacked bundle | Only if the process dies mid-install; removed on every failure `install` sees. |
| `<bundle>.old/` | `install` for an app in a macOS bundle, holding the bundle it replaced | Until the next `clean_old_binary`. |

`<exe>` is the whole executable path, so `app` and `app.v2` in one folder
never share these files. `<bundle>` is the whole bundle path, such as
`/Applications/App.app`. `clean_old_binary(&exe)` removes all of them;
`install` refuses to start while its staging file or folder exists.

## Timeouts and limits

| Constant | Value |
|---|---|
| `MANIFEST_TIMEOUT` | 3 s, for the manifest and for its signature |
| `DOWNLOAD_TIMEOUT` | 180 s, for the binary or a macOS app bundle's zip |
| `MAX_BINARY_BYTES` | 512 MiB |
| `MAX_APP_BYTES` | 1 GiB, for a macOS app bundle's zip |
