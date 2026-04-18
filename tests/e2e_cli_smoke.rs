//! End-to-end smoke test. Spawns `flarion serve` as a child process on a
//! temp port, then runs `flarion status --url ... --json` against it.
//!
//! Gated on FLARION_E2E=1 because CI port-binding on Windows is finicky.

#[cfg(unix)]
#[test]
fn e2e_serve_and_status_roundtrip() {
    if std::env::var("FLARION_E2E").is_err() {
        eprintln!("e2e_serve_and_status_roundtrip: skipping (set FLARION_E2E=1 to run)");
        return;
    }

    // Write a minimal server config.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        r#"
[server]
host = "127.0.0.1"
port = 18081
"#,
    ).unwrap();

    let bin = env!("CARGO_BIN_EXE_flarion");

    let mut server = std::process::Command::new(bin)
        .args(["serve", "-c"])
        .arg(tmp.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn flarion serve");

    // Wait for bind
    std::thread::sleep(std::time::Duration::from_secs(2));

    let out = std::process::Command::new(bin)
        .args(["status", "--url", "http://127.0.0.1:18081", "--json"])
        .output()
        .expect("run flarion status");

    // Clean up server before asserting so we don't leak a process on failure.
    let _ = server.kill();
    let _ = server.wait();

    assert!(
        out.status.success(),
        "status failed: status={:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert!(v.pointer("/server/version").is_some(), "missing /server/version: {v}");
    assert!(v.pointer("/gpus").is_some(), "missing /gpus");
    assert!(v.pointer("/models").is_some(), "missing /models");
}

#[cfg(windows)]
#[test]
fn e2e_smoke_skipped_on_windows() {
    // Windows CI port binding is finicky; the Unix test above covers the contract.
    eprintln!("e2e_smoke_skipped_on_windows: e2e test runs on Unix only (see CLAUDE.md)");
}
