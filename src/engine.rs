use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::manifest::Binary;
use crate::{DOWNLOAD_TIMEOUT, RenewError};

const CHUNK: usize = 64 * 1024;

pub fn staging_path(exe: &Path) -> PathBuf {
    beside(exe, ".staging")
}

fn old_path(exe: &Path) -> PathBuf {
    beside(exe, ".old")
}

fn beside(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(exe.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

pub fn install(binary: &Binary, exe: &Path) -> Result<(), RenewError> {
    let response = crate::get(&binary.url, DOWNLOAD_TIMEOUT, RenewError::DownloadFailed)?;
    let staging = staging_path(exe);
    let file = create_staging(&staging)?;
    let outcome = download_verified(binary, response.into_body().into_reader(), file)
        .and_then(|()| swap_in(exe, &staging));
    if outcome.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    outcome
}

#[cfg(unix)]
fn create_staging(staging: &Path) -> Result<File, RenewError> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(staging)
        .map_err(|source| RenewError::StageFailed { source })?;
    if let Err(source) = file.set_permissions(std::fs::Permissions::from_mode(0o755)) {
        drop(file);
        let _ = std::fs::remove_file(staging);
        return Err(RenewError::StageFailed { source });
    }
    Ok(file)
}

#[cfg(windows)]
fn create_staging(staging: &Path) -> Result<File, RenewError> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staging)
        .map_err(|source| RenewError::StageFailed { source })
}

fn download_verified(
    binary: &Binary,
    mut reader: impl Read,
    mut file: File,
) -> Result<(), RenewError> {
    let mut hasher = fetch::StreamHasher::new();
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
        if received > binary.size {
            return Err(RenewError::VerifyFailed {
                detail: format!("more than the published {} bytes arrived", binary.size),
            });
        }
        hasher.update(&buf[..got]);
        file.write_all(&buf[..got])
            .map_err(|source| RenewError::StageFailed { source })?;
    }
    file.sync_all()
        .map_err(|source| RenewError::StageFailed { source })?;
    drop(file);

    if received != binary.size {
        return Err(RenewError::VerifyFailed {
            detail: format!("{received} bytes arrived, {} were published", binary.size),
        });
    }
    let got_hex = hasher.finalize_hex();
    if !got_hex.eq_ignore_ascii_case(binary.sha256.trim()) {
        return Err(RenewError::VerifyFailed {
            detail: "the checksum does not match the published one".to_string(),
        });
    }
    Ok(())
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
    let _ = std::fs::remove_file(old_path(exe));
    let _ = std::fs::remove_file(staging_path(exe));
}

pub fn restart(exe: &Path) -> std::io::Result<()> {
    Command::new(exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> tempfile::TempDir {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/testscratch");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new()
            .prefix("renew-")
            .tempdir_in(root)
            .unwrap()
    }

    fn serve_once(body: Vec<u8>) -> String {
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

    fn binary_for(body: &[u8], url: String) -> Binary {
        Binary {
            url,
            sha256: fetch::sha256_hex(body),
            size: body.len() as u64,
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
        };

        assert!(matches!(
            install(&binary, &exe),
            Err(RenewError::DownloadFailed)
        ));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old binary");
    }

    #[test]
    fn a_plain_http_binary_url_off_this_machine_is_refused_before_anything_is_staged() {
        let dir = scratch();
        let exe = dir.path().join("pito-work");
        std::fs::write(&exe, b"old binary").unwrap();
        let binary = Binary {
            url: String::from("http://example.com/app"),
            sha256: "ab".repeat(32),
            size: 4,
        };

        assert!(matches!(
            install(&binary, &exe),
            Err(RenewError::InsecureUrl)
        ));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old binary");
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
