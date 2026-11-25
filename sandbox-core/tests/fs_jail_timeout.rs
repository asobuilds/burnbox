//! Milestone 11 integration coverage for the opt-in filesystem jail and the
//! wall-clock timeout. Both tests exercise the real published binary in its
//! unprivileged namespaces, so a host that denies mounts takes the documented
//! non-fatal fallback path instead of failing CI.

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_burnbox-core")
}

fn assert_contract_stdout(stdout: &[u8]) -> Value {
    let text = String::from_utf8_lossy(stdout);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "stdout must be two JSON lines: {text}");
    let started: Value = serde_json::from_str(lines[0]).expect("started JSON");
    let wiped: Value = serde_json::from_str(lines[1]).expect("wiped JSON");
    assert_eq!(started["status"], "started");
    assert_eq!(wiped["status"], "wiped");
    assert_eq!(started["mount_point"], wiped["mount_point"]);
    started
}

#[test]
fn fs_jail_hides_host_etc_but_exposes_staged_workspace() {
    let root = std::env::temp_dir().join(format!("burnbox-m11-jail-{}", std::process::id()));
    fs::create_dir_all(&root).expect("create test root");
    let staged = root.join("marker.txt");
    fs::write(&staged, b"staged-inside-ram").expect("write staged file");

    let output = Command::new(binary())
        .args(["--mode", "online", "--ram-mb", "64", "--fs-jail"])
        .arg("--copy-in")
        .arg(&staged)
        .args(["--target", "/bin/sh", "--", "-c"])
        .arg(
            "if [ -e /etc/hostname ]; then echo HOST_ETC_VISIBLE >&2; else echo HOST_ETC_ABSENT >&2; fi; \
             cat \"$HOME/marker.txt\" >&2",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run fs-jail session");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let started = assert_contract_stdout(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");

    let mount_point = started["mount_point"].as_str().expect("mount point");
    assert!(
        !Path::new(mount_point).exists(),
        "workspace leaked after exit: {mount_point}"
    );

    // Target diagnostics go to stderr so stdout stays exactly two JSON lines.
    // The staged file is always in the RAM workspace, whether the jail is
    // active or the host LSM forced the overlay fallback.
    assert!(
        stderr.contains("staged-inside-ram"),
        "staged workspace file unreadable: {stderr}"
    );

    if stderr.contains("fs-jail active") {
        assert!(
            stderr.contains("HOST_ETC_ABSENT"),
            "host /etc must be unreachable in the jail: {stderr}"
        );
    } else {
        assert!(
            stderr.contains("falling back to overlay behavior"),
            "a jail failure must only warn: {stderr}"
        );
        assert!(
            stderr.contains("HOST_ETC_VISIBLE"),
            "fallback path must stay compatible with viewers: {stderr}"
        );
    }

    fs::remove_dir_all(&root).ok();
}

#[test]
fn timeout_kills_sleeping_target_and_scrubs() {
    let started_at = Instant::now();
    let output = Command::new(binary())
        .args([
            "--mode",
            "online",
            "--ram-mb",
            "64",
            "--timeout",
            "1",
            "--target",
            "/bin/sh",
            "--",
            "-c",
            "sleep 30",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run timeout session");
    let elapsed = started_at.elapsed();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let started = assert_contract_stdout(&output.stdout);
    assert_ne!(output.status.code(), Some(0), "timeout must be non-zero");
    assert!(
        stderr.contains("timeout 1s elapsed"),
        "missing timeout diagnostic: {stderr}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "target outlived timeout + 1s: {elapsed:?}"
    );

    let mount_point = started["mount_point"].as_str().expect("mount point");
    assert!(
        !Path::new(mount_point).exists(),
        "timed-out workspace was not scrubbed: {mount_point}"
    );
}
