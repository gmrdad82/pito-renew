use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use renew::{AppBundle, Binary, RenewError};
use sha2::{Digest, Sha256};

const NEW_BINARY: &[u8] = b"the new binary, version 1.1.0";
const OLD_BINARY: &[u8] = b"the running binary, version 1.0.0";
const ISSUED: u64 = 1_790_000_000;
const SLUG: &str = "pito-work";
const BINARY_PATH: &str = "/pito-work-1.1.0";

struct Route {
    path: String,
    status: &'static str,
    body: Vec<u8>,
}

fn ok(path: &str, body: Vec<u8>) -> Route {
    Route {
        path: path.to_string(),
        status: "200 OK",
        body,
    }
}

struct Server {
    base: String,
    hits: Arc<AtomicUsize>,
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn manifest_url(&self) -> String {
        self.url("/manifest.json")
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

fn serve(routes: impl FnOnce(&str) -> Vec<Route>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes = routes(&base);
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else { continue };
            counter.fetch_add(1, Ordering::SeqCst);
            let mut head = [0u8; 4096];
            let got = socket.read(&mut head).unwrap_or(0);
            let request = String::from_utf8_lossy(&head[..got]);
            let path = request.split_whitespace().nth(1).unwrap_or("");
            let (status, body) = routes
                .iter()
                .find(|route| route.path == path)
                .map_or(("404 Not Found", &[][..]), |route| {
                    (route.status, route.body.as_slice())
                });
            let _ = write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(body);
        }
    });
    Server { base, hits }
}

fn closed_port_url() -> String {
    String::from("http://127.0.0.1:9/manifest.json")
}

fn keys() -> minisign::KeyPair {
    minisign::KeyPair::generate_unencrypted_keypair().unwrap()
}

fn sign(keys: &minisign::KeyPair, raw: &[u8]) -> Vec<u8> {
    minisign::sign(Some(&keys.pk), &keys.sk, raw, None, None)
        .unwrap()
        .into_string()
        .into_bytes()
}

fn sha256_hex(body: &[u8]) -> String {
    format!("{:x}", Sha256::digest(body))
}

struct Published {
    version: &'static str,
    issued: Option<u64>,
    app: Option<&'static str>,
}

impl Default for Published {
    fn default() -> Self {
        Self {
            version: "1.1.0",
            issued: Some(ISSUED),
            app: Some(SLUG),
        }
    }
}

impl Published {
    fn json(&self, base: &str) -> Vec<u8> {
        let mut extra = String::new();
        if let Some(issued) = self.issued {
            extra.push_str(&format!(r#","issued":{issued}"#));
        }
        if let Some(app) = self.app {
            extra.push_str(&format!(r#","app":"{app}""#));
        }
        format!(
            r#"{{"version":"{version}","platforms":{{"{key}":{{"url":"{base}{BINARY_PATH}","sha256":"{sha256}","size":{size}}}}}{extra}}}"#,
            version = self.version,
            key = renew::platform_key(),
            sha256 = sha256_hex(NEW_BINARY),
            size = NEW_BINARY.len(),
        )
        .into_bytes()
    }
}

fn release(keys: &minisign::KeyPair, published: Published) -> Server {
    serve(|base| {
        let raw = published.json(base);
        let signature = sign(keys, &raw);
        vec![
            ok("/manifest.json", raw),
            ok("/manifest.json.minisig", signature),
            ok(BINARY_PATH, NEW_BINARY.to_vec()),
        ]
    })
}

fn scratch() -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/testscratch");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("renew-story-")
        .tempdir_in(root)
        .unwrap()
}

fn running_exe(dir: &Path) -> PathBuf {
    let exe = dir.join("pito-work");
    std::fs::write(&exe, OLD_BINARY).unwrap();
    exe
}

fn old_of(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

fn assert_untouched(exe: &Path) {
    assert_eq!(std::fs::read(exe).unwrap(), OLD_BINARY);
    assert!(!renew::staging_path(exe).exists());
    assert!(!old_of(exe).exists());
}

#[test]
fn an_app_finds_an_update_installs_it_and_cleans_up_after_itself() {
    let keys = keys();
    let server = release(&keys, Published::default());
    let dir = scratch();
    let exe = running_exe(dir.path());

    let check = renew::check(&server.manifest_url(), &keys.pk.to_base64(), "1.0.0").unwrap();
    let update = check.update.expect("1.1.0 is an update for 1.0.0");
    assert_eq!(update.version, "1.1.0");
    assert_eq!(check.new_major, None);
    assert_eq!(check.issued, Some(ISSUED));

    renew::install(&update.binary, &exe).unwrap();

    assert_eq!(std::fs::read(&exe).unwrap(), NEW_BINARY);
    assert!(!renew::staging_path(&exe).exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::symlink_metadata(&exe).unwrap();
        assert!(meta.file_type().is_file());
        assert_eq!(meta.permissions().mode() & 0o777, 0o755);
        assert!(!old_of(&exe).exists());
    }
    #[cfg(windows)]
    assert_eq!(std::fs::read(old_of(&exe)).unwrap(), OLD_BINARY);
    #[cfg(unix)]
    std::fs::write(old_of(&exe), OLD_BINARY).unwrap();

    renew::clean_old_binary(&exe);

    assert_eq!(std::fs::read(&exe).unwrap(), NEW_BINARY);
    assert!(!old_of(&exe).exists());
    assert!(!renew::staging_path(&exe).exists());
}

#[test]
fn an_app_on_the_published_version_or_past_it_is_current() {
    let keys = keys();
    let server = release(&keys, Published::default());
    let key = keys.pk.to_base64();

    for current in ["1.1.0", "1.1.0+local", "1.2.0"] {
        let check = renew::check(&server.manifest_url(), &key, current).unwrap();
        assert_eq!(check.update, None, "{current} is current");
        assert_eq!(check.new_major, None);
    }
}

#[test]
fn check_since_passes_at_the_floor_and_refuses_a_manifest_issued_before_it() {
    let keys = keys();
    let server = release(&keys, Published::default());
    let key = keys.pk.to_base64();

    for floor in [0, ISSUED - 1, ISSUED] {
        let check = renew::check_since(&server.manifest_url(), &key, "1.0.0", floor).unwrap();
        assert_eq!(check.update.map(|r| r.version).as_deref(), Some("1.1.0"));
    }

    let outcome = renew::check_since(&server.manifest_url(), &key, "1.0.0", ISSUED + 1);
    assert!(matches!(
        outcome,
        Err(RenewError::StaleManifest { issued: Some(ISSUED), not_before }) if not_before == ISSUED + 1
    ));
}

#[test]
fn check_since_refuses_an_undated_manifest_whatever_the_floor() {
    let keys = keys();
    let server = release(
        &keys,
        Published {
            issued: None,
            ..Published::default()
        },
    );

    for floor in [0, ISSUED] {
        let outcome =
            renew::check_since(&server.manifest_url(), &keys.pk.to_base64(), "1.0.0", floor);
        assert!(matches!(
            outcome,
            Err(RenewError::StaleManifest { issued: None, not_before }) if not_before == floor
        ));
    }
    let plain = renew::check(&server.manifest_url(), &keys.pk.to_base64(), "1.0.0").unwrap();
    assert!(plain.update.is_some());
    assert_eq!(plain.issued, None);
}

#[test]
fn check_app_passes_for_its_own_slug_and_refuses_another_apps_manifest() {
    let keys = keys();
    let server = release(&keys, Published::default());
    let key = keys.pk.to_base64();

    let check = renew::check_app(&server.manifest_url(), &key, SLUG, "1.0.0", ISSUED).unwrap();
    assert_eq!(check.update.map(|r| r.version).as_deref(), Some("1.1.0"));

    let outcome = renew::check_app(&server.manifest_url(), &key, "pito-other", "1.0.0", 0);
    assert!(matches!(
        outcome,
        Err(RenewError::WrongApp { expected, found: Some(found) })
            if expected == "pito-other" && found == SLUG
    ));
}

#[test]
fn check_app_refuses_a_manifest_that_names_no_app() {
    let keys = keys();
    let server = release(
        &keys,
        Published {
            app: None,
            ..Published::default()
        },
    );

    let outcome = renew::check_app(
        &server.manifest_url(),
        &keys.pk.to_base64(),
        SLUG,
        "1.0.0",
        0,
    );
    assert!(matches!(
        outcome,
        Err(RenewError::WrongApp { found: None, .. })
    ));
}

#[test]
fn check_app_holds_the_floor_too() {
    let keys = keys();
    let server = release(&keys, Published::default());

    let outcome = renew::check_app(
        &server.manifest_url(),
        &keys.pk.to_base64(),
        SLUG,
        "1.0.0",
        ISSUED + 1,
    );
    assert!(matches!(
        outcome,
        Err(RenewError::StaleManifest {
            issued: Some(ISSUED),
            ..
        })
    ));
}

#[test]
fn a_closed_port_or_a_missing_manifest_is_unreachable_from_every_entry_point() {
    let key = keys().pk.to_base64();
    let nothing = serve(|_| Vec::new());

    for url in [closed_port_url(), nothing.manifest_url()] {
        assert!(matches!(
            renew::check(&url, &key, "1.0.0"),
            Err(RenewError::ManifestUnreachable)
        ));
        assert!(matches!(
            renew::check_since(&url, &key, "1.0.0", 0),
            Err(RenewError::ManifestUnreachable)
        ));
        assert!(matches!(
            renew::check_app(&url, &key, SLUG, "1.0.0", 0),
            Err(RenewError::ManifestUnreachable)
        ));
    }
}

#[test]
fn a_server_error_on_the_manifest_is_unreachable() {
    let key = keys().pk.to_base64();
    let server = serve(|_| {
        vec![Route {
            path: "/manifest.json".to_string(),
            status: "500 Internal Server Error",
            body: b"{}".to_vec(),
        }]
    });

    assert!(matches!(
        renew::check(&server.manifest_url(), &key, "1.0.0"),
        Err(RenewError::ManifestUnreachable)
    ));
}

#[test]
fn a_manifest_signed_by_another_key_is_refused_by_check_app() {
    let signer = keys();
    let server = release(&signer, Published::default());

    let outcome = renew::check_app(
        &server.manifest_url(),
        &keys().pk.to_base64(),
        SLUG,
        "1.0.0",
        0,
    );
    assert!(matches!(outcome, Err(RenewError::BadSignature)));
}

#[test]
fn a_manifest_rewritten_after_signing_is_refused() {
    let keys = keys();
    let server = serve(|base| {
        let raw = Published::default().json(base);
        let signature = sign(&keys, &raw);
        let tampered = String::from_utf8(raw)
            .unwrap()
            .replace(r#""version":"1.1.0""#, r#""version":"9.9.9""#)
            .into_bytes();
        vec![
            ok("/manifest.json", tampered),
            ok("/manifest.json.minisig", signature),
        ]
    });

    let outcome = renew::check_since(&server.manifest_url(), &keys.pk.to_base64(), "1.0.0", 0);
    assert!(matches!(outcome, Err(RenewError::BadSignature)));
}

#[test]
fn a_bad_public_key_is_refused_before_any_connection() {
    let keys = keys();
    let server = release(&keys, Published::default());
    let half = &keys.pk.to_base64()[..20];

    for key in ["", "not a key", half] {
        assert!(matches!(
            renew::check(&server.manifest_url(), key, "1.0.0"),
            Err(RenewError::BadPublicKey)
        ));
        assert!(matches!(
            renew::check_app(&server.manifest_url(), key, SLUG, "1.0.0", 0),
            Err(RenewError::BadPublicKey)
        ));
    }
    assert_eq!(server.hits(), 0);
}

#[test]
fn a_plain_http_manifest_off_this_machine_is_refused_as_insecure() {
    let key = keys().pk.to_base64();

    assert!(matches!(
        renew::check_since("http://example.com/manifest.json", &key, "1.0.0", 0),
        Err(RenewError::InsecureUrl)
    ));
    assert!(matches!(
        renew::check_app("http://example.com/manifest.json", &key, SLUG, "1.0.0", 0),
        Err(RenewError::InsecureUrl)
    ));
}

const MANIFEST_CAP: usize = 1024 * 1024;

fn manifest_of_exactly(keys: &minisign::KeyPair, len: usize) -> Server {
    serve(|base| {
        let mut raw = Published::default().json(base);
        raw.resize(len, b' ');
        let signature = sign(keys, &raw);
        vec![
            ok("/manifest.json", raw),
            ok("/manifest.json.minisig", signature),
        ]
    })
}

#[test]
fn a_manifest_one_byte_past_the_size_cap_is_refused_even_when_signed() {
    let keys = keys();
    let server = manifest_of_exactly(&keys, MANIFEST_CAP + 1);

    let outcome = renew::check(&server.manifest_url(), &keys.pk.to_base64(), "1.0.0");
    assert!(matches!(outcome, Err(RenewError::BadSignature)));
}

#[test]
fn a_manifest_exactly_at_the_size_cap_is_accepted() {
    let keys = keys();
    let server = manifest_of_exactly(&keys, MANIFEST_CAP);

    let check = renew::check(&server.manifest_url(), &keys.pk.to_base64(), "1.0.0").unwrap();
    assert!(check.update.is_some());
}

fn binary_served(status: &'static str, body: Vec<u8>) -> (Server, Binary) {
    let server = serve(|_| {
        vec![Route {
            path: BINARY_PATH.to_string(),
            status,
            body,
        }]
    });
    let binary = Binary {
        url: server.url(BINARY_PATH),
        sha256: sha256_hex(NEW_BINARY),
        size: NEW_BINARY.len() as u64,
        app: None,
    };
    (server, binary)
}

fn refused(binary: &Binary) -> RenewError {
    let dir = scratch();
    let exe = running_exe(dir.path());
    let error = renew::install(binary, &exe).expect_err("the install must be refused");
    assert_untouched(&exe);
    error
}

#[test]
fn more_bytes_than_published_are_refused_and_the_running_binary_stays() {
    let mut body = NEW_BINARY.to_vec();
    body.extend_from_slice(b" and a payload riding along");
    let (_server, binary) = binary_served("200 OK", body);

    assert!(matches!(refused(&binary), RenewError::VerifyFailed { .. }));
}

fn stream_endlessly(total: usize) -> (String, Receiver<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}{BINARY_PATH}", listener.local_addr().unwrap());
    let (sent, written) = channel();
    std::thread::spawn(move || {
        let Ok((mut socket, _)) = listener.accept() else {
            return;
        };
        let mut head = [0u8; 4096];
        let _ = socket.read(&mut head);
        let _ = write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
        );
        let chunk = [0xAAu8; 64 * 1024];
        let mut done = 0;
        while done < total {
            match socket.write(&chunk[..chunk.len().min(total - done)]) {
                Ok(0) | Err(_) => break,
                Ok(n) => done += n,
            }
        }
        let _ = sent.send(done);
    });
    (url, written)
}

#[test]
fn a_download_far_past_the_published_size_is_cut_off_long_before_it_ends() {
    const ENDLESS: usize = 256 * 1024 * 1024;
    let (url, written) = stream_endlessly(ENDLESS);
    let binary = Binary {
        url,
        sha256: sha256_hex(NEW_BINARY),
        size: NEW_BINARY.len() as u64,
        app: None,
    };

    assert!(matches!(refused(&binary), RenewError::VerifyFailed { .. }));
    let sent = written
        .recv_timeout(Duration::from_secs(10))
        .expect("the client hangs up once it refuses");
    assert!(sent < ENDLESS / 4, "{sent} of {ENDLESS} bytes were sent");
}

#[test]
fn a_missing_or_failing_download_is_a_download_failure() {
    let (missing, mut binary) = binary_served("200 OK", NEW_BINARY.to_vec());
    binary.url = missing.url("/not-published");
    assert!(matches!(refused(&binary), RenewError::DownloadFailed));

    let (_failing, binary) = binary_served("503 Service Unavailable", NEW_BINARY.to_vec());
    assert!(matches!(refused(&binary), RenewError::DownloadFailed));

    let binary = Binary {
        url: closed_port_url(),
        ..binary
    };
    assert!(matches!(refused(&binary), RenewError::DownloadFailed));
}

#[test]
fn a_caller_built_app_bundle_with_an_insane_size_is_refused_before_any_fetch() {
    let (server, binary) = binary_served("200 OK", NEW_BINARY.to_vec());

    for size in [0, renew::MAX_APP_BYTES + 1, u64::MAX] {
        let binary = Binary {
            app: Some(AppBundle {
                url: server.url("/pito-work.app.zip"),
                sha256: "ab".repeat(32),
                size,
            }),
            ..binary.clone()
        };
        assert!(matches!(refused(&binary), RenewError::VerifyFailed { .. }));
    }
    assert_eq!(server.hits(), 0);
}

#[test]
fn a_caller_built_app_bundle_with_a_malformed_checksum_is_refused_before_any_fetch() {
    let (server, binary) = binary_served("200 OK", NEW_BINARY.to_vec());
    let binary = Binary {
        app: Some(AppBundle {
            url: server.url("/pito-work.app.zip"),
            sha256: "not hex".to_string(),
            size: 10,
        }),
        ..binary
    };

    assert!(matches!(refused(&binary), RenewError::ManifestMalformed));
    assert_eq!(server.hits(), 0);
}

#[test]
fn a_caller_built_binary_with_a_zero_size_is_refused_before_any_fetch() {
    let (server, binary) = binary_served("200 OK", NEW_BINARY.to_vec());
    let binary = Binary { size: 0, ..binary };

    assert!(matches!(refused(&binary), RenewError::VerifyFailed { .. }));
    assert_eq!(server.hits(), 0);
}

#[test]
fn a_staging_file_left_by_a_crashed_run_blocks_until_clean_old_binary_sweeps_it() {
    let (server, binary) = binary_served("200 OK", NEW_BINARY.to_vec());
    let dir = scratch();
    let exe = running_exe(dir.path());
    let staging = renew::staging_path(&exe);
    std::fs::write(&staging, &NEW_BINARY[..7]).unwrap();

    assert!(matches!(
        renew::install(&binary, &exe),
        Err(RenewError::StageFailed { .. })
    ));
    assert_eq!(std::fs::read(&exe).unwrap(), OLD_BINARY);
    assert_eq!(std::fs::read(&staging).unwrap(), &NEW_BINARY[..7]);
    assert_eq!(server.hits(), 0);

    renew::clean_old_binary(&exe);
    assert!(!staging.exists());
    assert_eq!(std::fs::read(&exe).unwrap(), OLD_BINARY);

    renew::install(&binary, &exe).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), NEW_BINARY);
    assert!(!staging.exists());
    assert_eq!(server.hits(), 1);
}

#[test]
fn a_refused_install_can_be_retried_and_succeed() {
    let (_short, short) = binary_served("200 OK", NEW_BINARY[..5].to_vec());
    let (_whole, whole) = binary_served("200 OK", NEW_BINARY.to_vec());
    let dir = scratch();
    let exe = running_exe(dir.path());

    assert!(matches!(
        renew::install(&short, &exe),
        Err(RenewError::VerifyFailed { .. })
    ));
    assert_untouched(&exe);

    renew::install(&whole, &exe).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), NEW_BINARY);
}
