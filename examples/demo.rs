use std::io::{Read, Write};
use std::net::TcpListener;

use renew::{Check, Release, platform_key};

const CURRENT: &str = "1.0.0";
const LATEST: &str = "1.1.0";

fn serve(keys: &minisign::KeyPair, published: &[u8], served: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let manifest = format!(
        r#"{{"version":"{LATEST}","platforms":{{"{key}":{{"url":"{base}/app","sha256":"{sha}","size":{size}}}}}}}"#,
        key = platform_key(),
        sha = fetch::sha256_hex(published),
        size = published.len(),
    );
    let signature = minisign::sign(Some(&keys.pk), &keys.sk, manifest.as_bytes(), None, None)
        .unwrap()
        .into_string();
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else { continue };
            let mut head = [0u8; 4096];
            let got = socket.read(&mut head).unwrap_or(0);
            let request = String::from_utf8_lossy(&head[..got]);
            let body: &[u8] = if request.starts_with("GET /manifest.json ") {
                manifest.as_bytes()
            } else if request.starts_with("GET /manifest.json.minisig ") {
                signature.as_bytes()
            } else {
                &served
            };
            let _ = write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(body);
        }
    });
    base
}

fn main() {
    let tampered = std::env::args().any(|arg| arg == "--tampered");
    let published = format!("app {LATEST}\n").into_bytes();
    let served = if tampered {
        b"app 6.6.6\n".to_vec()
    } else {
        published.clone()
    };
    let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let base = serve(&keys, &published, served);

    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("app");
    std::fs::write(&exe, format!("app {CURRENT}\n")).unwrap();
    let platform = match platform_key() {
        "" => "an unlisted platform",
        key => key,
    };
    println!("running   app {CURRENT} on {platform}");

    let binary = match renew::check(
        &format!("{base}/manifest.json"),
        &keys.pk.to_base64(),
        CURRENT,
    ) {
        Ok(Check {
            update: Some(Release { version, binary }),
            ..
        }) => {
            println!("check     {version} is out, {} bytes", binary.size);
            binary
        }
        Ok(Check { update: None, .. }) => {
            println!("check     up to date");
            return;
        }
        Err(error) => {
            println!("check     {error}");
            return;
        }
    };

    match renew::install(&binary, &exe) {
        Ok(()) => println!("install   ✓ the sha256 matches, swapped in place"),
        Err(error) => println!("install   ✗ {error}"),
    }
    let on_disk = std::fs::read_to_string(&exe).unwrap();
    println!("on disk   {}", on_disk.trim_end());
    let leftover = if renew::staging_path(&exe).exists() {
        "a staging file"
    } else {
        "none"
    };
    println!("leftover  {leftover}");
}
