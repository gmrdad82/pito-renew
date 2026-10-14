# Inspiration

- [Sparkle](https://github.com/sparkle-project/Sparkle) 2.10.0 (MIT), the
  macOS app updater: renew's install of a whole `.app` bundle follows its
  `SUPlainInstaller`, `SUFileManager` and `SUCodeSigningVerifier` (unpack the
  zip with `ditto -x -k`, check the new bundle's code signature and that its
  Team ID matches the running app's, clear quarantine, touch the bundle, warm
  Gatekeeper with `gktool scan`, swap with `renamex_np(RENAME_SWAP)` and fall
  back to two renames that put the old bundle back on failure). Its notice is
  in NOTICE.
