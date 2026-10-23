use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::manifest::{Binary, sane};
use crate::{DOWNLOAD_TIMEOUT, MAX_APP_BYTES, MAX_BINARY_BYTES, RenewError};

const CHUNK: usize = 64 * 1024;

pub fn staging_path(exe: &Path) -> PathBuf {
    beside(exe, ".staging")
}

fn old_path(exe: &Path) -> PathBuf {
    beside(exe, ".old")
}

pub(crate) fn beside(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(exe.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

pub fn install(binary: &Binary, exe: &Path) -> Result<(), RenewError> {
    sane(&binary.sha256, binary.size, MAX_BINARY_BYTES)?;
    if let Some(app) = &binary.app {
        sane(&app.sha256, app.size, MAX_APP_BYTES)?;
    }
    #[cfg(target_os = "macos")]
    if let Some(bundle) = crate::bundle::bundle_of(exe) {
        return crate::bundle::install(binary, &bundle);
    }
    let staging = staging_path(exe);
    let mut file = create_staging(&staging)?;
    let outcome = crate::get(&binary.url, DOWNLOAD_TIMEOUT, RenewError::DownloadFailed)
        .and_then(|response| {
            download_verified(
                &binary.sha256,
                binary.size,
                response.into_body().into_reader(),
                &mut file,
            )
        })
        .and_then(|()| executable(&file))
        .and_then(|()| {
            drop(file);
            swap_in(exe, &staging)
        });
    if outcome.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    outcome
}

#[cfg(unix)]
fn create_staging(staging: &Path) -> Result<File, RenewError> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(staging)
        .map_err(|source| RenewError::StageFailed { source })
}

#[cfg(unix)]
fn executable(file: &File) -> Result<(), RenewError> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o755))
        .map_err(|source| RenewError::StageFailed { source })
}

#[cfg(windows)]
fn executable(_: &File) -> Result<(), RenewError> {
    Ok(())
}

#[cfg(windows)]
fn create_staging(staging: &Path) -> Result<File, RenewError> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staging)
        .map_err(|source| RenewError::StageFailed { source })
}

pub(crate) fn download_verified(
    sha256: &str,
    size: u64,
    mut reader: impl Read,
    file: &mut File,
) -> Result<(), RenewError> {
    let expected = digest_of(sha256).ok_or(RenewError::ManifestMalformed)?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut buf = [0u8; CHUNK];
    loop {
        let got = reader
            .read(&mut buf)
            .map_err(|_| RenewError::DownloadFailed)?;
        if got == 0 {
            break;
        }
        received += got as u64;
        if received > size {
            return Err(RenewError::VerifyFailed {
                detail: format!("more than the published {size} bytes arrived"),
            });
        }
        hasher.update(&buf[..got]);
        file.write_all(&buf[..got])
            .map_err(|source| RenewError::StageFailed { source })?;
    }
    file.sync_all()
        .map_err(|source| RenewError::StageFailed { source })?;

    if received != size {
        return Err(RenewError::VerifyFailed {
            detail: format!("{received} bytes arrived, {size} were published"),
        });
    }
    if hasher.finalize()[..] != expected[..] {
        return Err(RenewError::VerifyFailed {
            detail: "the checksum does not match the published one".to_string(),
        });
    }
    Ok(())
}

fn digest_of(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim().as_bytes();
    if hex.len() != 64 {
        return None;
    }
    let mut digest = [0u8; 32];
    for (byte, pair) in digest.iter_mut().zip(hex.chunks_exact(2)) {
        let high = (pair[0] as char).to_digit(16)?;
        let low = (pair[1] as char).to_digit(16)?;
        *byte = (high << 4 | low) as u8;
    }
    Some(digest)
}

#[cfg(unix)]
fn swap_in(exe: &Path, staging: &Path) -> Result<(), RenewError> {
    std::fs::rename(staging, exe).map_err(|source| RenewError::SwapFailed { source })?;
    if let Some(parent) = exe.parent()
        && let Ok(dir) = File::open(parent)
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

#[cfg(windows)]
fn swap_in(exe: &Path, staging: &Path) -> Result<(), RenewError> {
    let swap = || -> std::io::Result<()> {
        let old = old_path(exe);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(exe, &old)?;
        if let Err(error) = std::fs::rename(staging, exe) {
            let _ = std::fs::rename(&old, exe);
            return Err(error);
        }
        Ok(())
    };
    swap().map_err(|source| RenewError::SwapFailed { source })
}

pub fn clean_old_binary(exe: &Path) {
    #[cfg(target_os = "macos")]
    if let Some(bundle) = crate::bundle::bundle_of(exe) {
        crate::bundle::clean(&bundle);
    }
    let _ = std::fs::remove_file(old_path(exe));
    let _ = std::fs::remove_file(staging_path(exe));
}

pub fn restart(exe: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    if let Some(bundle) = crate::bundle::bundle_of(exe) {
        return crate::bundle::relaunch(&bundle);
    }
    Command::new(exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn scratch() -> tempfile::TempDir {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/testscratch");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new()
            .prefix("renew-")
            .tempdir_in(root)
            .unwrap()
    }

    pub(crate) fn serve_once(body: Vec<u8>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                let mut sink = [0u8; 4096];
                let _ = socket.read(&mut sink);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes());
                let _ = socket.write_all(&body);
            }
        });
        format!("http://{addr}/work")
    }

    pub(crate) fn sha256_hex(body: &[u8]) -> String {
        format!("{:x}", Sha256::digest(body))
    }

    fn binary_for(body: &[u8], url: String) -> Binary {
        Binary {
            url,
            sha256: sha256_hex(body),
            size: body.len() as u64,
            app: None,
        }
    }

    #[test]
    fn install_downloads_verifies_and_swaps_the_binary_in() {
        let dir = scratch();
        let exe = dir.path().join("pito-work");
        std::fs::write(&exe, b"old binary").unwrap();
        let body = b"the new binary bytes".to_vec();
        let url = serve_once(body.clone());
        let binary = binary_for(&body, url);

        install(&binary, &exe).unwrap();

        assert_eq!(std::fs::read(&exe).unwrap(), body);
        assert!(!staging_path(&exe).exists(), "no staging file left behind");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o777,
                0o755,
                "the new binary is 0755 whatever the umask"
            );
        }
    }

    #[test]
    fn a_checksum_mismatch_refuses_and_leaves_the_old_binary_untouched() {
        let dir = scratch();
        let exe = dir.path().join("pito-work");
        std::fs::write(&exe, b"old binary").unwrap();
        let body = b"tampered bytes".to_vec();
        let url = serve_once(body.clone());
        let mut binary = binary_for(&body, url);
        binary.sha256 = "0".repeat(64);

        let outcome = install(&binary, &exe);

        assert!(matches!(outcome, Err(RenewError::VerifyFailed { .. })));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old binary");
        assert!(!staging_path(&exe).exists());
    }

    #[test]
    fn a_size_mismatch_refuses_before_the_checksum_even_matters() {
        let dir = scratch();
        let exe = dir.path().join("pito-work");
        std::fs::write(&exe, b"old binary").unwrap();
        let body = b"short".to_vec();
        let url = serve_once(body.clone());
        let mut binary = binary_for(&body, url);
        binary.size += 5;

        let outcome = install(&binary, &exe);

        assert!(matches!(outcome, Err(RenewError::VerifyFailed { .. })));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old binary");
    }

    #[test]
    fn an_unreachable_url_is_a_download_failure_not_a_panic() {
        let dir = scratch();
        let exe = dir.path().join("pito-work");
        std::fs::write(&exe, b"old binary").unwrap();
        let url = String::from("https://renew.invalid/gone");
        let binary = Binary {
            url,
            sha256: "ab".repeat(32),
            size: 4,
            app: None,
        };

        assert!(matches!(
            install(&binary, &exe),
            Err(RenewError::DownloadFailed)
        ));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old binary");
    }

    #[test]
    fn a_plain_http_binary_url_off_this_machine_is_refused_and_nothing_is_left_staged() {
        let dir = scratch();
        let exe = dir.path().join("pito-work");
        std::fs::write(&exe, b"old binary").unwrap();
        let binary = Binary {
            url: String::from("http://example.com/app"),
            sha256: "ab".repeat(32),
            size: 4,
            app: None,
        };

        assert!(matches!(
            install(&binary, &exe),
            Err(RenewError::InsecureUrl)
        ));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old binary");
        assert!(!staging_path(&exe).exists());
    }

    #[test]
    fn a_caller_built_binary_over_the_cap_is_refused_before_anything_is_fetched() {
        let dir = scratch();
        let exe = dir.path().join("pito-work");
        std::fs::write(&exe, b"old binary").unwrap();
        let binary = Binary {
            url: String::from("http://127.0.0.1:9/app"),
            sha256: "ab".repeat(32),
            size: crate::MAX_BINARY_BYTES + 1,
            app: None,
        };

        assert!(matches!(
            install(&binary, &exe),
            Err(RenewError::VerifyFailed { .. })
        ));
        assert!(!staging_path(&exe).exists());
    }

    #[test]
    fn an_install_already_in_flight_is_refused_and_its_staging_file_is_left_alone() {
        let dir = scratch();
        let exe = dir.path().join("pito-work");
        std::fs::write(&exe, b"old binary").unwrap();
        std::fs::write(staging_path(&exe), b"in flight").unwrap();
        let body = b"the new binary bytes".to_vec();
        let url = serve_once(body.clone());
        let binary = binary_for(&body, url);

        let outcome = install(&binary, &exe);

        assert!(matches!(outcome, Err(RenewError::StageFailed { .. })));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old binary");
        assert_eq!(std::fs::read(staging_path(&exe)).unwrap(), b"in flight");
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_staging_symlink_is_refused_and_its_target_is_never_written() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let exe = dir.path().join("pito-work");
        std::fs::write(&exe, b"old binary").unwrap();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"victim bytes").unwrap();
        std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&victim, staging_path(&exe)).unwrap();
        let body = b"the new binary bytes".to_vec();
        let url = serve_once(body.clone());
        let binary = binary_for(&body, url);

        let outcome = install(&binary, &exe);

        assert!(matches!(outcome, Err(RenewError::StageFailed { .. })));
        assert_eq!(std::fs::read(&victim).unwrap(), b"victim bytes");
        let mode = std::fs::metadata(&victim).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read(&exe).unwrap(), b"old binary");
        assert!(
            std::fs::symlink_metadata(&exe)
                .unwrap()
                .file_type()
                .is_file()
        );
    }

    #[test]
    fn download_verified_refuses_a_checksum_of_the_wrong_length_before_reading() {
        let dir = scratch();
        let path = dir.path().join("staged");
        let mut file = File::create(&path).unwrap();
        let body = b"payload".to_vec();

        for sha in ["ab".repeat(31) + "a", "ab".repeat(33), String::new()] {
            let mut reader = std::io::Cursor::new(body.clone());
            let outcome = download_verified(&sha, body.len() as u64, &mut reader, &mut file);
            assert!(matches!(outcome, Err(RenewError::ManifestMalformed)));
            assert_eq!(reader.position(), 0);
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"");
    }

    #[test]
    fn download_verified_refuses_a_checksum_with_non_hex_digits() {
        let dir = scratch();
        let mut file = File::create(dir.path().join("staged")).unwrap();
        let body = b"payload";

        for sha in ["zz".repeat(32), "az".repeat(32)] {
            let outcome = download_verified(&sha, body.len() as u64, &body[..], &mut file);
            assert!(matches!(outcome, Err(RenewError::ManifestMalformed)));
        }
    }

    struct CutOff {
        left: Vec<u8>,
    }

    impl Read for CutOff {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.left.is_empty() {
                return Err(std::io::ErrorKind::ConnectionReset.into());
            }
            let n = buf.len().min(self.left.len());
            buf[..n].copy_from_slice(&self.left[..n]);
            self.left.drain(..n);
            Ok(n)
        }
    }

    #[test]
    fn a_connection_dropped_mid_body_is_a_download_failure() {
        let dir = scratch();
        let mut file = File::create(dir.path().join("staged")).unwrap();
        let body = b"the new binary bytes";
        let reader = CutOff {
            left: body[..8].to_vec(),
        };

        let outcome = download_verified(&sha256_hex(body), body.len() as u64, reader, &mut file);

        assert!(matches!(outcome, Err(RenewError::DownloadFailed)));
    }

    #[cfg(unix)]
    #[test]
    fn a_swap_that_fails_removes_the_verified_staging_file_and_leaves_the_target_alone() {
        let dir = scratch();
        let exe = dir.path().join("pito-work");
        std::fs::create_dir(&exe).unwrap();
        std::fs::write(exe.join("kept"), b"kept").unwrap();
        let body = b"the new binary bytes".to_vec();
        let url = serve_once(body.clone());
        let binary = binary_for(&body, url);

        let outcome = install(&binary, &exe);

        assert!(matches!(outcome, Err(RenewError::SwapFailed { .. })));
        assert!(!staging_path(&exe).exists());
        assert_eq!(std::fs::read(exe.join("kept")).unwrap(), b"kept");
    }

    #[test]
    fn sibling_binaries_with_one_stem_get_their_own_staging_and_old_paths() {
        let app = Path::new("bin").join("app");
        let v2 = Path::new("bin").join("app.v2");

        assert_eq!(staging_path(&v2), Path::new("bin").join("app.v2.staging"));
        assert_ne!(staging_path(&v2), staging_path(&app));
        assert_ne!(old_path(&v2), old_path(&app));
        assert_eq!(
            staging_path(Path::new("app.exe")),
            Path::new("app.exe.staging")
        );
    }

    #[test]
    fn clean_old_binary_sweeps_both_leftover_shapes() {
        let dir = scratch();
        let exe = dir.path().join("pito-work");
        std::fs::write(staging_path(&exe), b"staged").unwrap();
        std::fs::write(old_path(&exe), b"old").unwrap();

        clean_old_binary(&exe);

        assert!(!staging_path(&exe).exists());
        assert!(!old_path(&exe).exists());
    }

    #[cfg(unix)]
    #[test]
    fn restart_spawns_the_named_binary_detached() {
        assert!(restart(Path::new("/usr/bin/true")).is_ok());
        assert!(restart(Path::new("/definitely/not/a/binary")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn restart_spawns_the_named_binary_detached() {
        assert!(restart(Path::new("C:\\Windows\\System32\\cmd.exe")).is_ok());
        assert!(restart(Path::new("C:\\definitely\\not\\a\\binary.exe")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn an_unwritable_staging_location_is_a_stage_failure_not_a_download_failure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let sealed = dir.path().join("sealed");
        std::fs::create_dir(&sealed).unwrap();
        std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o555)).unwrap();
        let exe = sealed.join("pito-work");
        let body = b"new bytes".to_vec();
        let url = serve_once(body.clone());
        let binary = binary_for(&body, url);

        let outcome = install(&binary, &exe);

        std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(outcome, Err(RenewError::StageFailed { .. })));
    }

    #[cfg(windows)]
    #[test]
    fn a_failed_second_rename_rolls_the_old_binary_back_so_a_binary_always_exists() {
        let dir = scratch();
        let exe = dir.path().join("pito-work.exe");
        std::fs::write(&exe, b"running binary").unwrap();

        let outcome = swap_in(&exe, &staging_path(&exe));

        assert!(matches!(outcome, Err(RenewError::SwapFailed { .. })));
        assert_eq!(
            std::fs::read(&exe).unwrap(),
            b"running binary",
            "the swap must never leave the exe path empty"
        );
        assert!(!old_path(&exe).exists());
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_swap_parks_the_running_binary_as_old_then_renames_the_staging_in() {
        let dir = scratch();
        let exe = dir.path().join("pito-work.exe");
        std::fs::write(&exe, b"running binary").unwrap();
        std::fs::write(staging_path(&exe), b"new binary").unwrap();

        swap_in(&exe, &staging_path(&exe)).unwrap();

        assert_eq!(std::fs::read(&exe).unwrap(), b"new binary");
        assert_eq!(std::fs::read(old_path(&exe)).unwrap(), b"running binary");
    }
}
