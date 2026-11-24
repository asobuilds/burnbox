//! Integration test: interrupting a live session must never leave the RAM
//! workspace behind, and stdout must still contain the two JSON status lines.

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

#[test]
fn sigterm_mid_session_scrubs_workspace() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_burnbox-core"))
        .args([
            "--mode",
            "online",
            "--ram-mb",
            "8",
            "--target",
            "/bin/sleep",
            "--",
            "30",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn burnbox-core");

    let stdout = child.stdout.take().expect("stdout pipe");
    let mut lines = BufReader::new(stdout).lines();

    let started: Value = serde_json::from_str(
        &lines
            .next()
            .expect("started status line")
            .expect("read started line"),
    )
    .expect("started line is JSON");
    assert_eq!(started["status"], "started");
    let mount_point = started["mount_point"]
        .as_str()
        .expect("mount_point string")
        .to_string();
    assert!(
        Path::new(&mount_point).exists(),
        "workspace should exist while the session runs"
    );

    kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM).expect("send SIGTERM");

    let status = child.wait().expect("wait for burnbox-core");
    assert_eq!(status.code(), Some(1), "interrupted session exits 1");

    let wiped: Value = serde_json::from_str(
        &lines
            .next()
            .expect("wiped status line")
            .expect("read wiped line"),
    )
    .expect("wiped line is JSON");
    assert_eq!(wiped["status"], "wiped");
    assert_eq!(wiped["mount_point"], mount_point);

    assert!(
        !Path::new(&mount_point).exists(),
        "SIGTERM left workspace behind: {mount_point}"
    );
}
