#[cfg(any(target_os = "macos", test))]
mod bundle;
mod engine;
mod manifest;

pub use engine::{clean_old_binary, install, restart, staging_path};
pub use manifest::{AppBundle, Binary, Check, Release, platform_key};

use std::time::Duration;

pub const MANIFEST_TIMEOUT: Duration = Duration::from_secs(3);
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);
pub const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_APP_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RenewError {
    #[error("the release manifest could not be reached")]
    ManifestUnreachable,
    #[error("the release manifest did not parse")]
    ManifestMalformed,
    #[error("the public key is not a minisign public key")]
    BadPublicKey,
    #[error("the release manifest's signature does not verify")]
    BadSignature,
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
    #[error("refused a plain http url: only https, or http to this machine, is fetched")]
    InsecureUrl,
    #[error("the new binary could not be swapped in: {source}")]
    SwapFailed { source: std::io::Error },
    #[error("this app runs from a macOS .app bundle and no bundle is published for it")]
    NoAppBundle,
    #[error("{} is not writable by this user, so the app can't update itself", path.display())]
    NotWritable { path: std::path::PathBuf },
    #[error("the release manifest was issued before {not_before}, so it is stale or replayed")]
    StaleManifest {
        issued: Option<u64>,
        not_before: u64,
    },
}

pub fn check(
    manifest_url: &str,
    public_key: &str,
    current_version: &str,
) -> Result<Check, RenewError> {
    let public_key = manifest::public_key(public_key)?;
    let manifest = manifest::fetch_manifest(manifest_url, &public_key)?;
    manifest.check_against(current_version)
}

pub fn check_since(
    manifest_url: &str,
    public_key: &str,
    current_version: &str,
    not_before: u64,
) -> Result<Check, RenewError> {
    let public_key = manifest::public_key(public_key)?;
    let manifest = manifest::fetch_manifest(manifest_url, &public_key)?;
    manifest.issued_since(not_before)?;
    manifest.check_against(current_version)
}

pub(crate) fn get(
    url: &str,
    timeout: Duration,
    failed: RenewError,
) -> Result<ureq::http::Response<ureq::Body>, RenewError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .https_only(!loopback(url))
        .build()
        .into();
    match agent.get(url).call() {
        Ok(response) if response.status().is_success() => Ok(response),
        Err(ureq::Error::RequireHttpsOnly(_)) => Err(RenewError::InsecureUrl),
        _ => Err(failed),
    }
}

fn loopback(url: &str) -> bool {
    let Ok(uri) = url.parse::<ureq::http::Uri>() else {
        return false;
    };
    let Some(host) = uri.host() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    fn keys() -> minisign::KeyPair {
        minisign::KeyPair::generate_unencrypted_keypair().unwrap()
    }

    fn signed_manifest(keys: &minisign::KeyPair) -> (Vec<u8>, Vec<u8>) {
        let raw = format!(
            r#"{{"version":"1.1.0","platforms":{{"{key}":{{"url":"https://example.invalid/app","sha256":"{sha256}","size":100}}}}}}"#,
            key = platform_key(),
            sha256 = "ab".repeat(32),
        )
        .into_bytes();
        let signature = minisign::sign(Some(&keys.pk), &keys.sk, raw.as_slice(), None, None)
            .unwrap()
            .into_string()
            .into_bytes();
        (raw, signature)
    }

    fn serve(routes: Vec<(&'static str, Vec<u8>)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(mut socket) = socket else { continue };
                let mut head = [0u8; 4096];
                let got = socket.read(&mut head).unwrap_or(0);
                let request = String::from_utf8_lossy(&head[..got]);
                let path = request.split_whitespace().nth(1).unwrap_or("");
                let (status, body) = routes
                    .iter()
                    .find(|(route, _)| *route == path)
                    .map_or(("404 Not Found", &[][..]), |(_, body)| {
                        ("200 OK", body.as_slice())
                    });
                let _ = write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(body);
            }
        });
        format!("http://{addr}/manifest.json")
    }

    fn served(raw: Vec<u8>, signature: Vec<u8>) -> String {
        serve(vec![
            ("/manifest.json", raw),
            ("/manifest.json.minisig", signature),
        ])
    }

    #[test]
    fn a_signed_newer_manifest_is_available() {
        let keys = keys();
        let (raw, signature) = signed_manifest(&keys);
        let url = served(raw, signature);
        let update = check(&url, &keys.pk.to_base64(), "1.0.0")
            .unwrap()
            .update
            .expect("a newer signed version must be an update");
        assert_eq!(update.version, "1.1.0");
        assert_eq!(update.binary.size, 100);
    }

    #[test]
    fn a_flipped_byte_in_the_served_manifest_is_a_bad_signature() {
        let keys = keys();
        let (mut raw, signature) = signed_manifest(&keys);
        raw[0] ^= 1;
        let url = served(raw, signature);
        let outcome = check(&url, &keys.pk.to_base64(), "1.0.0");
        assert!(matches!(outcome, Err(RenewError::BadSignature)));
    }

    #[test]
    fn a_manifest_signed_by_another_key_is_a_bad_signature() {
        let (raw, signature) = signed_manifest(&keys());
        let url = served(raw, signature);
        let outcome = check(&url, &keys().pk.to_base64(), "1.0.0");
        assert!(matches!(outcome, Err(RenewError::BadSignature)));
    }

    #[test]
    fn a_malformed_signature_file_is_a_bad_signature() {
        let keys = keys();
        let (raw, _) = signed_manifest(&keys);
        let url = served(raw, b"not a signature".to_vec());
        let outcome = check(&url, &keys.pk.to_base64(), "1.0.0");
        assert!(matches!(outcome, Err(RenewError::BadSignature)));
    }

    #[test]
    fn a_missing_signature_is_an_unreachable_manifest() {
        let keys = keys();
        let (raw, _) = signed_manifest(&keys);
        let url = serve(vec![("/manifest.json", raw)]);
        let outcome = check(&url, &keys.pk.to_base64(), "1.0.0");
        assert!(matches!(outcome, Err(RenewError::ManifestUnreachable)));
    }

    #[test]
    fn a_garbage_public_key_is_refused_before_any_fetch() {
        let outcome = check("http://127.0.0.1:9/manifest.json", "not a key", "1.0.0");
        assert!(matches!(outcome, Err(RenewError::BadPublicKey)));
    }

    #[test]
    fn plain_http_to_another_host_is_refused() {
        let outcome = check("http://example.com/m.json", &keys().pk.to_base64(), "1.0.0");
        assert!(matches!(outcome, Err(RenewError::InsecureUrl)));
    }
}
