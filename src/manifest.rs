use std::collections::HashMap;
use std::io::Read;

use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;

use crate::{MANIFEST_TIMEOUT, MAX_BINARY_BYTES, RenewError};

const MANIFEST_BYTES: u64 = 1024 * 1024;
const SIGNATURE_BYTES: u64 = 4 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Manifest {
    #[serde(flatten)]
    latest: Entry,
    #[serde(default)]
    releases: Vec<Entry>,
}

#[derive(Debug, Clone, Deserialize)]
struct Entry {
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
pub struct Release {
    pub version: String,
    pub binary: Binary,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Check {
    pub update: Option<Release>,
    pub new_major: Option<Release>,
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
        let current = stripped(current_version).ok_or(RenewError::BadVersion)?;
        let mut entries = Vec::with_capacity(1 + self.releases.len());
        for entry in std::iter::once(&self.latest).chain(&self.releases) {
            let version = stripped(&entry.version).ok_or(RenewError::ManifestMalformed)?;
            entries.push((version, entry));
        }
        entries.sort_by(|a, b| b.0.cmp(&a.0));
        let new_major = entries
            .iter()
            .filter(|(version, _)| version.major > current.major)
            .find_map(|(_, entry)| entry.release().ok());
        if new_major.is_some() {
            return Ok(Check {
                update: new_major.clone(),
                new_major,
            });
        }
        let update = entries
            .iter()
            .find(|(version, _)| version.major == current.major && *version > current)
            .map(|(_, entry)| entry.release())
            .transpose()?;
        Ok(Check {
            update,
            new_major: None,
        })
    }
}

fn stripped(version: &str) -> Option<semver::Version> {
    let mut version: semver::Version = version.parse().ok()?;
    version.build = semver::BuildMetadata::EMPTY;
    Some(version)
}

impl Entry {
    fn release(&self) -> Result<Release, RenewError> {
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
        Ok(Release {
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

    fn entry(version: &str, key: &str) -> String {
        format!(
            r#""version":"{version}","platforms":{{"{key}":{{"url":"https://example.invalid/{version}","sha256":"{sha256}","size":100}}}}"#,
            sha256 = "ab".repeat(32),
        )
    }

    fn lines(latest: &str, releases: &[&str]) -> Manifest {
        let releases: Vec<String> = releases
            .iter()
            .map(|version| format!("{{{}}}", entry(version, platform_key())))
            .collect();
        let raw = format!(
            r#"{{{},"releases":[{}]}}"#,
            entry(latest, platform_key()),
            releases.join(",")
        );
        Manifest::parse(raw.as_bytes()).unwrap()
    }

    fn versions(check: &Check) -> (Option<&str>, Option<&str>) {
        (
            check
                .update
                .as_ref()
                .map(|release| release.version.as_str()),
            check
                .new_major
                .as_ref()
                .map(|release| release.version.as_str()),
        )
    }

    #[test]
    fn the_update_crosses_majors_and_new_major_names_the_crossing() {
        let manifest = lines("2.0.0", &["1.5.0", "1.5.1", "1.4.3", "0.9.9"]);
        for (running, update, new_major) in [
            ("1.4.2", Some("2.0.0"), Some("2.0.0")),
            ("1.5.1", Some("2.0.0"), Some("2.0.0")),
            ("1.0.0-rc.3", Some("2.0.0"), Some("2.0.0")),
            ("2.0.0-rc.1", Some("2.0.0"), None),
            ("2.0.0", None, None),
            ("0.9.0", Some("2.0.0"), Some("2.0.0")),
        ] {
            let check = manifest.check_against(running).unwrap();
            assert_eq!(versions(&check), (update, new_major), "running {running}");
        }
    }

    #[test]
    fn the_newest_of_several_later_majors_is_the_update_and_its_binary_rides_along() {
        let check = lines("3.1.0", &["2.4.0", "3.0.2"])
            .check_against("1.2.0")
            .unwrap();
        assert_eq!(versions(&check), (Some("3.1.0"), Some("3.1.0")));
        assert_eq!(check.update.unwrap().binary.size, 100);
    }

    #[test]
    fn a_later_major_without_this_platform_falls_back_to_the_running_majors_update() {
        let raw = format!(
            r#"{{{},"releases":[{{{}}}]}}"#,
            entry("2.0.0", "definitely-not-this-platform"),
            entry("1.5.1", platform_key()),
        );
        let check = Manifest::parse(raw.as_bytes())
            .unwrap()
            .check_against("1.4.2")
            .unwrap();
        assert_eq!(versions(&check), (Some("1.5.1"), None));
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
            assert_eq!(check, Check::default(), "published {published}");
        }
    }

    #[test]
    fn build_metadata_never_makes_a_version_newer() {
        for (published, running) in [("1.2.0+rebuild", "1.2.0"), ("1.2.0", "1.2.0+local")] {
            let check = manifest(published, platform_key(), 100)
                .check_against(running)
                .unwrap();
            assert_eq!(check, Check::default(), "{published} against {running}");
        }
    }

    #[test]
    fn a_prerelease_current_version_still_compares_by_semver() {
        let check = manifest("1.0.0", platform_key(), 100)
            .check_against("1.0.0-rc.1")
            .unwrap();
        assert!(check.update.is_some());
    }

    #[test]
    fn a_newer_version_without_this_platform_is_a_named_refusal() {
        let outcome = manifest("1.1.0", "definitely-not-this-platform", 100).check_against("1.0.0");
        assert!(matches!(outcome, Err(RenewError::NoPlatformBinary)));
    }

    #[test]
    fn a_zero_or_absurd_published_size_is_refused() {
        for size in [0, MAX_BINARY_BYTES + 1] {
            let outcome = manifest("1.1.0", platform_key(), size).check_against("1.0.0");
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
                manifest_with("1.1.0", platform_key(), &sha256, 100).check_against("1.0.0");
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
        let outcome = manifest("1.1.0", platform_key(), 100).check_against("not-semver");
        assert!(matches!(outcome, Err(RenewError::BadVersion)));
    }

    #[test]
    fn an_unparseable_manifest_version_is_a_malformed_manifest() {
        let outcome = manifest("not-semver", platform_key(), 100).check_against("1.0.0");
        assert!(matches!(outcome, Err(RenewError::ManifestMalformed)));
    }
}
