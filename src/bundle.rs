use std::path::{Path, PathBuf};

use crate::RenewError;

pub(crate) fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let exe = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let named = |path: &Path, name: &str| path.file_name().is_some_and(|found| found == name);
    let app = bundle
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
    (named(macos, "MacOS") && named(contents, "Contents") && app).then(|| bundle.to_path_buf())
}

pub(crate) fn the_app_in(dir: &Path) -> Result<PathBuf, RenewError> {
    let refused = || RenewError::VerifyFailed {
        detail: "the bundle archive does not hold exactly one .app".to_string(),
    };
    let mut apps = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|source| RenewError::StageFailed { source })? {
        let entry = entry.map_err(|source| RenewError::StageFailed { source })?;
        if entry.file_name() == "__MACOSX" {
            continue;
        }
        let path = entry.path();
        let directory = entry.file_type().is_ok_and(|kind| kind.is_dir());
        let app = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
        if !directory || !app {
            return Err(refused());
        }
        apps.push(path);
    }
    match <[PathBuf; 1]>::try_from(apps) {
        Ok([app]) => Ok(app),
        Err(_) => Err(refused()),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Identity {
    identifier: String,
    team: String,
}

pub(crate) fn identity(codesign_display: &str) -> Option<Identity> {
    let field = |name: &str| {
        codesign_display
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .map(str::trim)
            .filter(|value| {
                !value.is_empty()
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            })
            .map(str::to_string)
    };
    Some(Identity {
        identifier: field("Identifier")?,
        team: field("TeamIdentifier")?,
    })
}

pub(crate) fn requirement(running: &Identity) -> String {
    format!(
        "=anchor apple generic and identifier \"{}\" and certificate leaf[subject.OU] = \"{}\"",
        running.identifier, running.team
    )
}

pub(crate) fn same_signer(running: &Identity, found: &Identity) -> Result<(), RenewError> {
    if found.team != running.team {
        return Err(RenewError::VerifyFailed {
            detail: format!(
                "the new bundle is signed by team {}, the running app by team {}",
                found.team, running.team
            ),
        });
    }
    if found.identifier != running.identifier {
        return Err(RenewError::VerifyFailed {
            detail: format!(
                "the new bundle is {}, the running app is {}",
                found.identifier, running.identifier
            ),
        });
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) use macos::{clean, install, relaunch};

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::CString;
    use std::fs::File;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use super::{Identity, identity, requirement, same_signer, the_app_in};
    use crate::engine::beside;
    use crate::manifest::{AppBundle, Binary};
    use crate::{DOWNLOAD_TIMEOUT, RenewError};

    const CODESIGN: &str = "/usr/bin/codesign";
    const DITTO: &str = "/usr/bin/ditto";
    const GKTOOL: &str = "/usr/bin/gktool";
    const OPEN: &str = "/usr/bin/open";
    const XATTR: &str = "/usr/bin/xattr";

    pub(crate) fn staging_dir(bundle: &Path) -> PathBuf {
        beside(bundle, ".staging")
    }

    fn old_dir(bundle: &Path) -> PathBuf {
        beside(bundle, ".old")
    }

    pub(crate) fn install(binary: &Binary, bundle: &Path) -> Result<(), RenewError> {
        let app = binary.app.as_ref().ok_or(RenewError::NoAppBundle)?;
        let parent = bundle.parent().unwrap_or(Path::new("/"));
        writable(parent)?;
        writable(bundle)?;
        let running = signed_by(bundle)?;
        staged(app, bundle, &running)
    }

    pub(crate) fn staged(
        app: &AppBundle,
        bundle: &Path,
        running: &Identity,
    ) -> Result<(), RenewError> {
        let staging = staging_dir(bundle);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&staging)
            .map_err(|source| RenewError::StageFailed { source })?;
        let outcome = stage_and_swap(app, bundle, &staging, running);
        if outcome.is_err() && bundle.exists() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        outcome
    }

    fn stage_and_swap(
        app: &AppBundle,
        bundle: &Path,
        staging: &Path,
        running: &Identity,
    ) -> Result<(), RenewError> {
        let stage = |source| RenewError::StageFailed { source };
        let response = crate::get(&app.url, DOWNLOAD_TIMEOUT, RenewError::DownloadFailed)?;
        let zip = staging.join("app.zip");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&zip)
            .map_err(stage)?;
        crate::engine::download_verified(
            &app.sha256,
            app.size,
            response.into_body().into_reader(),
            &mut file,
        )?;
        drop(file);
        let unpacked = staging.join("unpacked");
        std::fs::create_dir(&unpacked).map_err(stage)?;
        run(
            Command::new(DITTO)
                .arg("-x")
                .arg("-k")
                .arg(&zip)
                .arg(&unpacked),
            "the bundle archive did not unpack",
        )?;
        let _ = std::fs::remove_file(&zip);
        let new = the_app_in(&unpacked)?;
        same_signer(running, &signed_by(&new)?)?;
        run(
            Command::new(CODESIGN)
                .args(["--verify", "--deep", "--strict", "-R"])
                .arg(requirement(running))
                .arg(&new),
            "the new bundle's code signature does not verify",
        )?;
        prepare(&new);
        swap(bundle, &new)?;
        if let Some(parent) = bundle.parent()
            && let Ok(dir) = File::open(parent)
        {
            let _ = dir.sync_all();
        }
        let old = old_dir(bundle);
        let _ = std::fs::remove_dir_all(&old);
        let _ = std::fs::rename(staging, &old);
        Ok(())
    }

    fn writable(path: &Path) -> Result<(), RenewError> {
        let not_writable = || RenewError::NotWritable {
            path: path.to_path_buf(),
        };
        let path_c = CString::new(path.as_os_str().as_bytes()).map_err(|_| not_writable())?;
        if unsafe { libc::access(path_c.as_ptr(), libc::W_OK) } == 0 {
            Ok(())
        } else {
            Err(not_writable())
        }
    }

    fn signed_by(bundle: &Path) -> Result<Identity, RenewError> {
        let output = Command::new(CODESIGN)
            .args(["-d", "--verbose=2"])
            .arg(bundle)
            .stdin(Stdio::null())
            .output()
            .map_err(|source| RenewError::StageFailed { source })?;
        identity(&String::from_utf8_lossy(&output.stderr)).ok_or_else(|| RenewError::VerifyFailed {
            detail: format!("{} carries no Team ID to match", bundle.display()),
        })
    }

    fn run(command: &mut Command, refusal: &str) -> Result<(), RenewError> {
        let output = command
            .stdin(Stdio::null())
            .output()
            .map_err(|source| RenewError::StageFailed { source })?;
        if output.status.success() {
            return Ok(());
        }
        let said = String::from_utf8_lossy(&output.stderr);
        let said = said.lines().rev().find(|line| !line.trim().is_empty());
        Err(RenewError::VerifyFailed {
            detail: match said {
                Some(said) => format!("{refusal}: {}", said.trim()),
                None => refusal.to_string(),
            },
        })
    }

    fn prepare(new: &Path) {
        let quiet = |command: &mut Command| {
            let _ = command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        };
        quiet(
            Command::new(XATTR)
                .args(["-d", "-r", "com.apple.quarantine"])
                .arg(new),
        );
        if let Ok(dir) = File::open(new) {
            let _ = dir.set_modified(std::time::SystemTime::now());
        }
        if Path::new(GKTOOL).exists() {
            quiet(Command::new(GKTOOL).arg("scan").arg(new));
        }
    }

    pub(crate) fn swap(bundle: &Path, new: &Path) -> Result<(), RenewError> {
        let swap_failed = |source| RenewError::SwapFailed { source };
        let invalid = || swap_failed(std::io::Error::from(std::io::ErrorKind::InvalidInput));
        let from = CString::new(new.as_os_str().as_bytes()).map_err(|_| invalid())?;
        let to = CString::new(bundle.as_os_str().as_bytes()).map_err(|_| invalid())?;
        if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) } == 0 {
            return Ok(());
        }
        let parked = parked(new);
        std::fs::rename(bundle, &parked).map_err(swap_failed)?;
        if let Err(error) = std::fs::rename(new, bundle) {
            let _ = std::fs::rename(&parked, bundle);
            return Err(swap_failed(error));
        }
        Ok(())
    }

    fn parked(new: &Path) -> PathBuf {
        new.with_file_name(".previous.app")
    }

    pub(crate) fn clean(bundle: &Path) {
        let _ = std::fs::remove_dir_all(old_dir(bundle));
        let _ = std::fs::remove_dir_all(staging_dir(bundle));
    }

    pub(crate) fn relaunch(bundle: &Path) -> std::io::Result<()> {
        Command::new(OPEN)
            .arg("-n")
            .arg(bundle)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_executable_in_contents_macos_of_a_dot_app_is_in_a_bundle() {
        for (exe, bundle) in [
            (
                "/Applications/Splicer.app/Contents/MacOS/splicer",
                Some("/Applications/Splicer.app"),
            ),
            (
                "/Volumes/Tools/My Tool.APP/Contents/MacOS/tool",
                Some("/Volumes/Tools/My Tool.APP"),
            ),
            ("/usr/local/bin/splicer", None),
            ("/Applications/Splicer.app/Contents/Resources/splicer", None),
            ("/opt/Splicer/Contents/MacOS/splicer", None),
            ("splicer", None),
        ] {
            assert_eq!(
                bundle_of(Path::new(exe)),
                bundle.map(PathBuf::from),
                "{exe}"
            );
        }
    }

    #[test]
    fn the_archive_must_hold_exactly_one_app_directory() {
        let dir = crate::engine::tests::scratch();
        let root = dir.path();
        assert!(matches!(
            the_app_in(root),
            Err(RenewError::VerifyFailed { .. })
        ));
        std::fs::create_dir(root.join("__MACOSX")).unwrap();
        std::fs::create_dir(root.join("Splicer.app")).unwrap();
        assert_eq!(the_app_in(root).unwrap(), root.join("Splicer.app"));
        std::fs::create_dir(root.join("Other.app")).unwrap();
        assert!(matches!(
            the_app_in(root),
            Err(RenewError::VerifyFailed { .. })
        ));
        std::fs::remove_dir(root.join("Other.app")).unwrap();
        std::fs::write(root.join("stray"), b"x").unwrap();
        assert!(matches!(
            the_app_in(root),
            Err(RenewError::VerifyFailed { .. })
        ));
        std::fs::remove_file(root.join("stray")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("Splicer.app"), root.join("Linked.app")).unwrap();
            assert!(matches!(
                the_app_in(root),
                Err(RenewError::VerifyFailed { .. })
            ));
        }
    }

    #[test]
    fn a_signer_must_carry_a_team_id_and_match_the_running_app() {
        let display = |identifier: &str, team: &str| {
            format!(
                "Executable=/A.app/Contents/MacOS/a\nIdentifier={identifier}\nFormat=app bundle with Mach-O thin (arm64)\nAuthority=Developer ID Application: X (TEAM)\nTeamIdentifier={team}\n"
            )
        };
        let running = identity(&display("com.pitomd.splicer", "AB12CD34EF")).unwrap();
        assert!(identity(&display("com.pitomd.splicer", "not set")).is_none());
        assert!(identity("Identifier=com.pitomd.splicer\n").is_none());
        assert!(identity(&display("com.pitomd.splicer\" or true", "AB12CD34EF")).is_none());
        assert_eq!(
            requirement(&running),
            "=anchor apple generic and identifier \"com.pitomd.splicer\" and certificate leaf[subject.OU] = \"AB12CD34EF\""
        );
        assert!(same_signer(&running, &running.clone()).is_ok());
        for (identifier, team) in [
            ("com.pitomd.splicer", "ZZ99ZZ99ZZ"),
            ("com.pitomd.other", "AB12CD34EF"),
        ] {
            let found = identity(&display(identifier, team)).unwrap();
            assert!(matches!(
                same_signer(&running, &found),
                Err(RenewError::VerifyFailed { .. })
            ));
        }
    }

    #[cfg(target_os = "macos")]
    fn fake_bundle(at: &Path, marker: &[u8]) -> PathBuf {
        let bundle = at.join("Work.app");
        let macos = bundle.join("Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(macos.join("work"), marker).unwrap();
        bundle
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_unsigned_bundle_is_refused_and_the_running_one_stays_whole() {
        let dir = crate::engine::tests::scratch();
        let installed = dir.path().join("installed");
        let built = dir.path().join("built");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::create_dir_all(&built).unwrap();
        let bundle = fake_bundle(&installed, b"running");
        let new = fake_bundle(&built, b"new");
        let zip = dir.path().join("Work.zip");
        let status = std::process::Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(&new)
            .arg(&zip)
            .status()
            .unwrap();
        assert!(status.success());
        let body = std::fs::read(&zip).unwrap();
        let app = crate::manifest::AppBundle {
            url: crate::engine::tests::serve_once(body.clone()),
            sha256: crate::engine::tests::sha256_hex(&body),
            size: body.len() as u64,
        };
        let running = identity("Identifier=work\nTeamIdentifier=AB12CD34EF\n").unwrap();

        let outcome = macos::staged(&app, &bundle, &running);

        assert!(matches!(outcome, Err(RenewError::VerifyFailed { .. })));
        assert_eq!(
            std::fs::read(bundle.join("Contents/MacOS/work")).unwrap(),
            b"running"
        );
        assert!(!macos::staging_dir(&bundle).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_swap_trades_the_two_bundles_in_one_step() {
        let dir = crate::engine::tests::scratch();
        let installed = dir.path().join("installed");
        let staged = dir.path().join("staged");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::create_dir_all(&staged).unwrap();
        let bundle = fake_bundle(&installed, b"running");
        let new = fake_bundle(&staged, b"new");

        macos::swap(&bundle, &new).unwrap();

        assert_eq!(
            std::fs::read(bundle.join("Contents/MacOS/work")).unwrap(),
            b"new"
        );
        assert_eq!(
            std::fs::read(new.join("Contents/MacOS/work")).unwrap(),
            b"running"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_app_in_a_folder_it_cannot_write_says_so_before_any_download() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::engine::tests::scratch();
        let sealed = dir.path().join("sealed");
        std::fs::create_dir(&sealed).unwrap();
        let bundle = fake_bundle(&sealed, b"running");
        std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o555)).unwrap();
        let binary = crate::manifest::Binary {
            url: "https://renew.invalid/work".to_string(),
            sha256: "ab".repeat(32),
            size: 4,
            app: Some(crate::manifest::AppBundle {
                url: "https://renew.invalid/Work.zip".to_string(),
                sha256: "ab".repeat(32),
                size: 4,
            }),
        };

        let outcome = crate::install(&binary, &bundle.join("Contents/MacOS/work"));

        std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            matches!(&outcome, Err(RenewError::NotWritable { path }) if *path == std::fs::canonicalize(&sealed).unwrap()),
            "{outcome:?}"
        );
    }
}
