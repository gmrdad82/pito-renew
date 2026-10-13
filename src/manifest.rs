use std::collections::HashMap;
use std::io::Read;

use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;

use crate::{MANIFEST_TIMEOUT, MAX_BINARY_BYTES, RenewError};

const MANIFEST_BYTES: u64 = 1024 * 1024;
const SIGNATURE_BYTES: u64 = 4 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Manifest {
    version: String,
    platforms: HashMap<String, Binary>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Binary {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Check {
    Current,
    Available { version: String, binary: Binary },
}

pub fn platform_key() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x86_64",
        ("macos", "aarch64") => "macos-arm64",
        ("windows", "x86_64") => "windows-x86_64",
        _ => "",
    }
}

impl Manifest {
    pub(crate) fn verify(
        raw: &[u8],
        signature: &[u8],
        public_key: &PublicKey,
    ) -> Result<Self, RenewError> {
        let signature = std::str::from_utf8(signature)
            .ok()
            .and_then(|text| Signature::decode(text).ok())
            .ok_or(RenewError::BadSignature)?;
        public_key
            .verify(raw, &signature, false)
            .map_err(|_| RenewError::BadSignature)?;
        Self::parse(raw)
    }

    fn parse(raw: &[u8]) -> Result<Self, RenewError> {
        serde_json::from_slice(raw).map_err(|_| RenewError::ManifestMalformed)
    }

    pub(crate) fn check_against(&self, current_version: &str) -> Result<Check, RenewError> {
        let mut current: semver::Version = current_version
            .parse()
            .map_err(|_| RenewError::BadVersion)?;
        let mut latest: semver::Version = self
            .version
            .parse()
            .map_err(|_| RenewError::ManifestMalformed)?;
        current.build = semver::BuildMetadata::EMPTY;
        latest.build = semver::BuildMetadata::EMPTY;
        if latest <= current {
            return Ok(Check::Current);
        }
        let binary = self
            .platforms
            .get(platform_key())
            .ok_or(RenewError::NoPlatformBinary)?;
        if binary.sha256.len() != 64 || !binary.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(RenewError::ManifestMalformed);
        }
        if binary.size == 0 || binary.size > MAX_BINARY_BYTES {
            return Err(RenewError::VerifyFailed {
                detail: format!("published size {} is outside the sane band", binary.size),
            });
        }
        Ok(Check::Available {
            version: self.version.clone(),
            binary: binary.clone(),
        })
    }
}

pub(crate) fn public_key(base64: &str) -> Result<PublicKey, RenewError> {
    PublicKey::from_base64(base64.trim()).map_err(|_| RenewError::BadPublicKey)
}

pub(crate) fn fetch_manifest(url: &str, public_key: &PublicKey) -> Result<Manifest, RenewError> {
    let raw = fetch_capped(url, MANIFEST_BYTES)?;
    let signature = fetch_capped(&format!("{url}.minisig"), SIGNATURE_BYTES)?;
    Manifest::verify(&raw, &signature, public_key)
}

fn fetch_capped(url: &str, cap: u64) -> Result<Vec<u8>, RenewError> {
    let response = crate::get(url, MANIFEST_TIMEOUT, RenewError::ManifestUnreachable)?;
    let mut raw = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(cap)
        .read_to_end(&mut raw)
        .map_err(|_| RenewError::ManifestUnreachable)?;
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_with(version: &str, key: &str, sha256: &str, size: u64) -> Manifest {
        let raw = format!(
            r#"{{"version":"{version}","platforms":{{"{key}":{{"url":"https://example.invalid/work","sha256":"{sha256}","size":{size}}}}}}}"#
        );
        Manifest::parse(raw.as_bytes()).unwrap()
    }

    fn manifest(version: &str, key: &str, size: u64) -> Manifest {
        manifest_with(version, key, &"ab".repeat(32), size)
    }

    #[test]
    fn platform_key_matches_this_build_target() {
        let key = platform_key();
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => assert_eq!(key, "linux-x86_64"),
            ("macos", "aarch64") => assert_eq!(key, "macos-arm64"),
            ("windows", "x86_64") => assert_eq!(key, "windows-x86_64"),
            _ => assert_eq!(key, ""),
        }
    }

    #[test]
    fn the_same_or_an_older_version_is_current() {
        for published in ["1.1.0", "1.0.9", "0.9.0"] {
            let check = manifest(published, platform_key(), 100)
                .check_against("1.1.0")
                .unwrap();
            assert_eq!(check, Check::Current, "published {published}");
        }
    }

    #[test]
    fn build_metadata_never_makes_a_version_newer() {
        for (published, running) in [("1.2.0+rebuild", "1.2.0"), ("1.2.0", "1.2.0+local")] {
            let check = manifest(published, platform_key(), 100)
                .check_against(running)
                .unwrap();
            assert_eq!(check, Check::Current, "{published} against {running}");
        }
    }

    #[test]
    fn a_prerelease_current_version_still_compares_by_semver() {
        let check = manifest("1.0.0", platform_key(), 100)
            .check_against("1.0.0-rc.1")
            .unwrap();
        assert!(matches!(check, Check::Available { .. }));
    }

    #[test]
    fn a_newer_version_without_this_platform_is_a_named_refusal() {
        let outcome = manifest("2.0.0", "definitely-not-this-platform", 100).check_against("1.0.0");
        assert!(matches!(outcome, Err(RenewError::NoPlatformBinary)));
    }

    #[test]
    fn a_zero_or_absurd_published_size_is_refused() {
        for size in [0, MAX_BINARY_BYTES + 1] {
            let outcome = manifest("2.0.0", platform_key(), size).check_against("1.0.0");
            assert!(matches!(outcome, Err(RenewError::VerifyFailed { .. })));
        }
    }

    #[test]
    fn a_sha256_that_is_not_64_hex_digits_is_a_malformed_manifest() {
        for sha256 in [
            "a".repeat(63),
            "a".repeat(65),
            format!("{}g", "a".repeat(63)),
        ] {
            let outcome =
                manifest_with("2.0.0", platform_key(), &sha256, 100).check_against("1.0.0");
            assert!(
                matches!(outcome, Err(RenewError::ManifestMalformed)),
                "sha256 {sha256}"
            );
        }
    }

    #[test]
    fn garbage_bytes_are_a_malformed_manifest_not_a_panic() {
        assert!(matches!(
            Manifest::parse(b"not json"),
            Err(RenewError::ManifestMalformed)
        ));
        assert!(matches!(
            Manifest::parse(b"{}"),
            Err(RenewError::ManifestMalformed)
        ));
    }

    #[test]
    fn an_unparseable_current_version_is_a_bad_version_not_a_bad_manifest() {
        let outcome = manifest("2.0.0", platform_key(), 100).check_against("not-semver");
        assert!(matches!(outcome, Err(RenewError::BadVersion)));
    }

    #[test]
    fn an_unparseable_manifest_version_is_a_malformed_manifest() {
        let outcome = manifest("not-semver", platform_key(), 100).check_against("1.0.0");
        assert!(matches!(outcome, Err(RenewError::ManifestMalformed)));
    }
}
