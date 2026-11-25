//! Integration tests for rootless online networking.
//!
//! Cover the required missing-binary warning, the spawn/invocation path with a
//! deterministic stub, and (when a real slirp4netns is available) an end-to-end
//! assertion that the sandbox network namespace gains the slirp tap device.

use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const WARNING: &str = "Warning: slirp4netns not found; network isolation is strict loopback only. Install slirp4netns for live URL fetching.";

fn run_with_timeout(child: std::process::Child) -> std::process::Output {
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

fn base_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_burnbox-core"));
    command.env_remove("BURNBOX_SLIRP4NETNS");
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    command
}

fn assert_two_json_lines(output: &std::process::Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "expected two JSON lines, got: {stdout}");
    let started: Value = serde_json::from_str(lines[0]).expect("started JSON");
    let wiped: Value = serde_json::from_str(lines[1]).expect("wiped JSON");
    assert_eq!(started["status"], "started");
    assert_eq!(wiped["status"], "wiped");
    assert_eq!(wiped["mount_point"], started["mount_point"]);
    let mount = started["mount_point"]
        .as_str()
        .expect("mount_point string")
        .to_string();
    assert!(!Path::new(&mount).exists(), "workspace leaked: {mount}");
    started
}

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("burnbox-slirp-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

#[test]
fn missing_slirp_warns_and_continues() {
    let output = run_with_timeout(
        base_command()
            // A PATH with no slirp4netns anywhere.
            .env("PATH", "/nonexistent")
            .args(["--mode", "online", "--ram-mb", "16", "--target", "/bin/true"])
            .spawn()
            .expect("spawn burnbox-core"),
    );
    assert_eq!(output.status.code(), Some(0), "session should survive a missing helper");
    let started = assert_two_json_lines(&output);
    assert!(started["pid"].is_i64());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(WARNING), "missing warning absent: {stderr}");
    assert!(!stderr.contains("slirp4netns ready"), "unexpected ready message");
    // Absence is detected by PATH lookup only: no spawn, no privileged setup,
    // no package-manager attempt.
    assert!(
        !stderr.contains("could not start slirp4netns"),
        "a missing helper must not be spawned: {stderr}"
    );
    assert!(
        !stderr.contains("apt") && !stderr.contains("sudo"),
        "no package install or privilege escalation may be attempted: {stderr}"
    );
}

#[test]
fn absent_slirp_with_no_override_never_touches_network_setup() {
    // Explicitly cover the documented default state: helper absent and no
    // BURNBOX_SLIRP4NETNS override. The loopback-only warning is the entire
    // network action; the session itself succeeds.
    let output = run_with_timeout(
        base_command()
            .env("PATH", "/nonexistent")
            .args(["--mode", "online", "--ram-mb", "16", "--target", "/bin/true"])
            .spawn()
            .expect("spawn burnbox-core"),
    );
    assert_eq!(output.status.code(), Some(0));
    assert_two_json_lines(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.lines().any(|line| line == WARNING),
        "expected the exact loopback warning: {stderr}"
    );
    assert!(!stderr.contains("ready on burnbox0"), "network was configured: {stderr}");
}

#[test]
fn slirp_stub_is_invoked_with_supervisor_pid_and_tap() {
    let dir = temp_dir("stub");
    let binary = dir.join("slirp4netns");
    let marker = dir.join("args.txt");
    // The stub records its arguments, emits the readiness byte on stdout, then
    // stays alive so burnbox-core must tear it down at session end.
    fs::write(
        &binary,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"{}\"\nprintf x\nexec sleep 30\n",
            marker.display()
        ),
    )
    .expect("write stub");
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).expect("chmod stub");

    let output = run_with_timeout(
        base_command()
            .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
            .args(["--mode", "online", "--ram-mb", "16", "--target", "/bin/true"])
            .spawn()
            .expect("spawn burnbox-core"),
    );
    assert_eq!(output.status.code(), Some(0));

    let started = assert_two_json_lines(&output);
    let supervisor_pid = started["pid"].as_i64().expect("pid").to_string();
    let args = fs::read_to_string(&marker).expect("stub recorded its arguments");
    for expected in ["--configure", "--ready-fd=1", "burnbox0", &supervisor_pid] {
        assert!(args.contains(expected), "slirp args {args:?} missing {expected:?}");
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains(WARNING), "helper was found: {stderr}");
    assert!(stderr.contains("slirp4netns ready"), "no ready message: {stderr}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn real_slirp_configures_tap_in_sandbox() {
    // Self-skipping: CI hosts without slirp4netns must not fail.
    let helper = std::env::var_os("BURNBOX_SLIRP4NETNS")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("slirp4netns"))
                .find(|candidate| {
                    fs::metadata(candidate)
                        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                        .unwrap_or(false)
                })
        }));
    let Some(helper) = helper else {
        eprintln!("skipping: no slirp4netns binary available");
        return;
    };

    let dir = temp_dir("real");
    let marker = dir.join("interfaces.txt");
    let script = format!(
        "#!/bin/sh\ngrep burnbox0 /proc/net/dev > \"{}\"\nip -o -4 addr show burnbox0 >> \"{}\" 2>/dev/null || true\n",
        marker.display(),
        marker.display()
    );
    let output = run_with_timeout(
        base_command()
            .env("BURNBOX_SLIRP4NETNS", &helper)
            .args(["--mode", "online", "--ram-mb", "64", "--target", "/bin/sh"])
            .args(["--", "-c", &script])
            .spawn()
            .expect("spawn burnbox-core"),
    );
    assert_eq!(output.status.code(), Some(0));
    assert_two_json_lines(&output);
    let interfaces = fs::read_to_string(&marker).expect("interface snapshot");
    assert!(interfaces.contains("burnbox0"), "tap missing: {interfaces}");
    assert!(interfaces.contains("10.0.2.100"), "tap unconfigured: {interfaces}");
    let _ = fs::remove_dir_all(dir);
}
