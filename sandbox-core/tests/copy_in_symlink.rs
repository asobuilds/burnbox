//! Integration tests for `--copy-in` symlink hardening.
//!
//! A host symlink must never be followed into the sandbox: the only allowed
//! outcomes are an explicit rejection or staging the link itself without
//! resolving it. BurnBox chooses rejection, because a link copied verbatim
//! into the workspace could still be followed inside the sandbox (where host
//! `/etc` remains visible) and leak the same host contents.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio, Output};
use std::sync::mpsc;
use std::time::Duration;

const HOST_SECRET: &str = "burnbox-symlink-canary-9f2c";

fn run_with_timeout(child: std::process::Child) -> Output {
    let pid = child.id();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(result) => result.expect("wait for burnbox-core"),
        Err(_) => {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
            panic!("burnbox-core timed out");
        }
    }
}

fn burnbox() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_burnbox-core"));
    command.env_remove("BURNBOX_SLIRP4NETNS");
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    command
}

fn temp_root(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("burnbox-copyin-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create temp root");
    path
}

/// Assert the unchanged two-line JSON contract and return the workspace path.
fn assert_status_lines(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout must be exactly two JSON lines: {stdout}");
    let started: Value = serde_json::from_str(lines[0]).expect("started JSON");
    let wiped: Value = serde_json::from_str(lines[1]).expect("wiped JSON");
    assert_eq!(started["status"], "started");
    assert_eq!(wiped["status"], "wiped");
    assert_eq!(wiped["pid"], started["pid"]);
    assert_eq!(wiped["mount_point"], started["mount_point"]);
    started["mount_point"]
        .as_str()
        .expect("mount_point string")
        .to_owned()
}

#[test]
fn symlink_copy_in_is_rejected_without_leaking_host_contents() {
    let root = temp_root("symlink");
    let secret = root.join("host_secret");
    let link = root.join("evil_link");
    fs::write(&secret, HOST_SECRET).expect("write host canary");
    std::os::unix::fs::symlink(&secret, &link).expect("create symlink");

    let output = run_with_timeout(
        burnbox()
            .args([
                "--mode",
                "offline",
                "--ram-mb",
                "16",
                "--copy-in",
            ])
            .arg(&link)
            .args(["--target", "/bin/cat", "--", "evil_link"])
            .spawn()
            .expect("spawn burnbox-core"),
    );

    // Rejection is a runtime error, but the status/scrub contract still holds.
    assert_eq!(output.status.code(), Some(1), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let workspace = assert_status_lines(&output);
    assert!(!Path::new(&workspace).exists(), "workspace was not scrubbed: {workspace}");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--copy-in"), "no copy-in diagnostic: {stderr}");
    assert!(stderr.contains("symlink"), "symlink was not refused: {stderr}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains(HOST_SECRET), "host canary leaked to stdout");
    // Offline mode must not even consult the network helper.
    assert!(!stderr.contains("slirp4netns"), "offline mode touched networking: {stderr}");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn intermediate_directory_symlink_is_rejected() {
    let root = temp_root("parent-link");
    let real_dir = root.join("real");
    let link_dir = root.join("linkdir");
    fs::create_dir_all(&real_dir).expect("create real dir");
    fs::write(real_dir.join("data.txt"), HOST_SECRET).expect("write host canary");
    std::os::unix::fs::symlink(&real_dir, &link_dir).expect("create directory symlink");

    let output = run_with_timeout(
        burnbox()
            .args(["--mode", "offline", "--ram-mb", "16", "--copy-in"])
            .arg(link_dir.join("data.txt"))
            .args(["--target", "/bin/cat", "--", "data.txt"])
            .spawn()
            .expect("spawn burnbox-core"),
    );
    assert_eq!(output.status.code(), Some(1));
    assert_status_lines(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("symlink"), "parent symlink was not refused: {stderr}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains(HOST_SECRET), "host canary leaked");
    fs::remove_dir_all(&root).ok();
}

#[test]
fn regular_copy_in_still_stages_and_scrubs() {
    let root = temp_root("regular");
    let source = root.join("ticket.txt");
    fs::write(&source, format!("{HOST_SECRET}\n")).expect("write ticket");

    let output = run_with_timeout(
        burnbox()
            .args(["--mode", "offline", "--ram-mb", "16", "--copy-in"])
            .arg(&source)
            .args(["--target", "/bin/cat", "--", "ticket.txt"])
            .spawn()
            .expect("spawn burnbox-core"),
    );
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "expected started + file + wiped: {stdout}");
    assert_eq!(lines[1], HOST_SECRET, "staged file contents changed");
    let started: Value = serde_json::from_str(lines[0]).expect("started JSON");
    let wiped: Value = serde_json::from_str(lines[2]).expect("wiped JSON");
    assert_eq!(wiped["status"], "wiped");
    assert_eq!(wiped["mount_point"], started["mount_point"]);
    let workspace = started["mount_point"].as_str().expect("mount_point");
    assert!(!Path::new(workspace).exists(), "workspace leaked: {workspace}");
    fs::remove_dir_all(&root).ok();
}
