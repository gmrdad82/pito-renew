use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use renew::{Check, platform_key};

const SIZE: usize = 64 * 1024 * 1024;
const CHECKS: usize = 100;
const INSTALLS: usize = 7;

fn serve(manifest: String, signature: String, binary: Arc<Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else { continue };
            let manifest = manifest.clone();
            let signature = signature.clone();
            let binary = binary.clone();
            std::thread::spawn(move || {
                let mut head = [0u8; 4096];
                let got = socket.read(&mut head).unwrap_or(0);
                let request = String::from_utf8_lossy(&head[..got]);
                let body: &[u8] = if request.starts_with("GET /manifest.json ") {
                    manifest.as_bytes()
                } else if request.starts_with("GET /manifest.json.minisig ") {
                    signature.as_bytes()
                } else {
                    &binary
                };
                let _ = write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(body);
            });
        }
    });
    base
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

fn mib_per_s(time: Duration) -> f64 {
    SIZE as f64 / (1024.0 * 1024.0) / time.as_secs_f64()
}

fn main() {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let binary: Vec<u8> = (0..SIZE)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let sha = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&binary));
    let binary = Arc::new(binary);
    let downloads = serve(String::new(), String::new(), binary.clone());
    let manifest = format!(
        r#"{{"version":"1.1.0","platforms":{{"{key}":{{"url":"{downloads}/app","sha256":"{sha}","size":{SIZE}}}}}}}"#,
        key = platform_key(),
    );
    let signature = minisign::sign(Some(&keys.pk), &keys.sk, manifest.as_bytes(), None, None)
        .unwrap()
        .into_string();
    let base = serve(manifest, signature, binary.clone());
    let url = format!("{base}/manifest.json");
    let public_key = keys.pk.to_base64();

    let mut checks = Vec::with_capacity(CHECKS);
    let mut found = None;
    for _ in 0..CHECKS {
        let start = Instant::now();
        let Check { update, .. } = renew::check(&url, &public_key, "1.0.0").unwrap();
        checks.push(start.elapsed());
        found = update;
    }
    let release = found.expect("the manifest offers 1.1.0");

    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("app");
    let mut installs = Vec::with_capacity(INSTALLS);
    for _ in 0..INSTALLS {
        std::fs::write(&exe, b"app 1.0.0\n").unwrap();
        let start = Instant::now();
        renew::install(&release.binary, &exe).unwrap();
        installs.push(start.elapsed());
    }

    let out = dir.path().join("curl-app");
    let mut shells = Vec::with_capacity(INSTALLS);
    for _ in 0..INSTALLS {
        let start = Instant::now();
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "curl -fsS -o '{out}' '{url}' && sha256sum '{out}' >/dev/null && chmod 755 '{out}'",
                out = out.display(),
                url = release.binary.url,
            ))
            .status()
            .unwrap();
        assert!(status.success());
        shells.push(start.elapsed());
    }

    let start = Instant::now();
    let _ = format!(
        "{:x}",
        <sha2::Sha256 as sha2::Digest>::digest(binary.as_slice())
    );
    let hash = start.elapsed();

    let check = median(checks);
    let install = median(installs);
    let shell = median(shells);
    println!(
        "check    {:.2} ms   median of {CHECKS}: manifest + signature over loopback, verified",
        check.as_secs_f64() * 1000.0
    );
    println!(
        "install  {:.0} ms   median of {INSTALLS}: 64 MiB streamed, hashed, size-checked, swapped ({:.0} MiB/s)",
        install.as_secs_f64() * 1000.0,
        mib_per_s(install)
    );
    println!(
        "shell    {:.0} ms   median of {INSTALLS}: curl -o, then sha256sum, then chmod ({:.0} MiB/s)",
        shell.as_secs_f64() * 1000.0,
        mib_per_s(shell)
    );
    println!(
        "sha256   {:.0} ms   64 MiB in memory, once ({:.0} MiB/s)",
        hash.as_secs_f64() * 1000.0,
        mib_per_s(hash)
    );
}
