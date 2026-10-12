mod engine;
mod manifest;

pub use engine::{clean_old_binary, install, restart, staging_path};
pub use manifest::{Binary, Check, Manifest, platform_key};

use std::time::Duration;

pub const MANIFEST_TIMEOUT: Duration = Duration::from_secs(3);
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);
pub const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RenewError {
    #[error("the release manifest could not be reached")]
    ManifestUnreachable,
    #[error("the release manifest did not parse")]
    ManifestMalformed,
    #[error("no binary is published for this platform")]
    NoPlatformBinary,
    #[error("the download did not arrive whole")]
    DownloadFailed,
    #[error("a version string did not parse")]
    BadVersion,
    #[error("the downloaded binary could not be staged: {source}")]
    StageFailed { source: std::io::Error },
    #[error("the downloaded binary failed verification: {detail}")]
    VerifyFailed { detail: String },
    #[error("the new binary could not be swapped in: {source}")]
    SwapFailed { source: std::io::Error },
}

pub fn check(manifest_url: &str, current_version: &str) -> Result<Check, RenewError> {
    let manifest = manifest::fetch_manifest(manifest_url, MANIFEST_TIMEOUT)?;
    manifest.check_against(current_version)
}
