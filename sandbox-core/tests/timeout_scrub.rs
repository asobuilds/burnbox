//! Milestone 11.1 regression: a wall-clock timeout must run the parent-side
//! zeroize+rmdir scrub and still emit the `wiped` status line.

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn burnbox_entries() -> BTreeSet<String> {
    fs::read_dir("/dev/shm")
        .expect("read /dev/shm")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("burnbox-"))
        .collect()
}

#[test]
fn timeout_never_leaks_workspace_and_keeps_json_contract() {
    let before = burnbox_entries();
    let started_at = Instant::now();

    let output = Command::new(env!("CARGO_BIN_EXE_burnbox-core"))
        .args([
            "--mode",
            "offline",
            "--ram-mb",
            "512",
            "--timeout",
            "1",
            "--target",
            "/bin/sleep",
            "--",
            "30",
        ])
        .output()
        .expect("run timed-out session");

    let elapsed = started_at.elapsed();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_ne!(output.status.code(), Some(0), "timeout must exit non-zero");
    assert!(
        stderr.contains("timeout 1s elapsed"),
        "missing timeout diagnostic: {stderr}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "target outlived timeout + 1s: {elapsed:?}"
    );

    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout must be two JSON lines: {stdout}");
    let started: Value = serde_json::from_str(lines[0]).expect("started JSON");
    let wiped: Value = serde_json::from_str(lines[1]).expect("wiped JSON");
    assert_eq!(started["status"], "started");
    assert_eq!(wiped["status"], "wiped", "timeout path omitted wiped: {stdout}");
    assert_eq!(started["mount_point"], wiped["mount_point"]);
    assert_eq!(started["pid"], wiped["pid"]);

    let mount_point = started["mount_point"].as_str().expect("mount point string");
    assert!(
        !Path::new(mount_point).exists(),
        "timed-out workspace was not scrubbed: {mount_point}"
    );

    let after = burnbox_entries();
    let leaked: Vec<&String> = after.difference(&before).collect();
    assert!(
        leaked.is_empty(),
        "timeout leaked /dev/shm workspaces: {leaked:?}"
    );
}
