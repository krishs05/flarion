use std::process::Command;

#[test]
fn flarion_version_command_prints_version() {
    let bin = env!("CARGO_BIN_EXE_flarion");
    let out = std::process::Command::new(bin)
        .args(["version"])
        .output()
        .expect("run flarion version");
    assert!(out.status.success(), "exit: {:?} stderr: {}", out.status, String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")), "expected version in output: {stdout}");
    assert!(stdout.contains("FLARION") || stdout.contains("flarion"), "expected brand: {stdout}");
}

#[test]
fn bare_flarion_prints_hint_and_exits_2() {
    let bin = env!("CARGO_BIN_EXE_flarion");
    let out = Command::new(bin).output().expect("run flarion");
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2, got: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no subcommand") || stderr.contains("TUI"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn flarion_serve_help_lists_config_flag() {
    let bin = env!("CARGO_BIN_EXE_flarion");
    let out = Command::new(bin)
        .args(["serve", "--help"])
        .output()
        .expect("run flarion serve --help");
    assert!(out.status.success(), "serve --help should succeed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--config") || stdout.contains("-c"),
        "serve --help missing --config flag: {stdout}"
    );
}

#[test]
fn flarion_status_help_listed() {
    let bin = env!("CARGO_BIN_EXE_flarion");
    let out = Command::new(bin)
        .args(["--help"])
        .output()
        .expect("run flarion --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("serve"),
        "--help should list 'serve' subcommand: {stdout}"
    );
    assert!(
        stdout.contains("status"),
        "--help should list 'status' subcommand: {stdout}"
    );
}

#[test]
fn flarion_completions_bash_emits_script() {
    let bin = env!("CARGO_BIN_EXE_flarion");
    let out = std::process::Command::new(bin)
        .args(["completions", "bash"])
        .output().unwrap();
    assert!(out.status.success(), "exit: {:?} stderr: {}", out.status, String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("_flarion") || stdout.contains("flarion"),
        "expected bash completion content: {stdout}");
}
