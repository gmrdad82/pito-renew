use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use crate::{MAX_BINARY_BYTES, RenewError};

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub platforms: HashMap<String, Binary>,
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
    pub fn parse(raw: &[u8]) -> Result<Self, RenewError> {
        serde_json::from_slice(raw).map_err(|_| RenewError::ManifestMalformed)
    }

    pub fn check_against(&self, current_version: &str) -> Result<Check, RenewError> {
        let current: semver::Version = current_version
            .parse()
            .map_err(|_| RenewError::BadVersion)?;
        let latest: semver::Version = self
            .version
            .parse()
            .map_err(|_| RenewError::ManifestMalformed)?;
        if latest <= current {
            return Ok(Check::Current);
        }
        let binary = self
            .platforms
            .get(platform_key())
            .ok_or(RenewError::NoPlatformBinary)?;
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

pub(crate) fn fetch_manifest(url: &str, timeout: Duration) -> Result<Manifest, RenewError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build()
        .into();
    let response = agent
        .get(url)
        .call()
        .map_err(|_| RenewError::ManifestUnreachable)?;
    if !response.status().is_success() {
        return Err(RenewError::ManifestUnreachable);
    }
    let mut raw = Vec::new();
    use std::io::Read;
    response
        .into_body()
        .into_reader()
        .take(1024 * 1024)
        .read_to_end(&mut raw)
        .map_err(|_| RenewError::ManifestUnreachable)?;
    Manifest::parse(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str, key: &str, size: u64) -> Manifest {
        let raw = format!(
            r#"{{"version":"{version}","platforms":{{"{key}":{{"url":"https://example.invalid/work","sha256":"ab","size":{size}}}}}}}"#
        );
        Manifest::parse(raw.as_bytes()).unwrap()
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
    fn a_newer_version_with_a_platform_binary_is_available() {
        let check = manifest("1.2.0", platform_key(), 100)
            .check_against("1.1.0")
            .unwrap();
        match check {
            Check::Available { version, binary } => {
                assert_eq!(version, "1.2.0");
                assert_eq!(binary.size, 100);
            }
            Check::Current => panic!("a newer version must be Available"),
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
