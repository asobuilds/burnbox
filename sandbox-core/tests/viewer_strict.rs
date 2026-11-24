//! Integration test: a real viewer opening a tiny local file under the strict
//! seccomp allow-list must succeed and keep every BurnBox contract intact.
//!
//! The test is intentionally self-skipping: headless viewers are not part of
//! the standard Rust toolchain, so CI hosts without one must still pass.

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Check the ELF magic so we never select a shell wrapper (such as the snap
/// `/usr/bin/firefox`) that cannot run inside the user namespace.
fn is_elf(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .map(|_| magic == [0x7f, b'E', b'L', b'F'])
        .unwrap_or(false)
}

/// Find a native (ELF) Firefox binary. The explicit environment override is
/// useful for packagers; the well-known paths cover snap, distro, and PATH
/// installations without ever selecting a non-ELF launcher.
fn find_viewer() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(path) = std::env::var("BURNBOX_VIEWER") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend(
        [
            "/snap/firefox/current/usr/lib/firefox/firefox",
            "/usr/lib/firefox/firefox",
            "/usr/lib/firefox-esr/firefox-esr",
            "/opt/firefox/firefox",
        ]
        .iter()
        .map(PathBuf::from),
    );
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            for name in ["firefox", "firefox-bin"] {
                if let Ok(canonical) = Path::new(dir).join(name).canonicalize() {
                    candidates.push(canonical);
                }
            }
        }
    }
    candidates
        .into_iter()
        .find(|path| path.is_file() && is_elf(path))
}

#[test]
fn strict_seccomp_viewer_opens_tiny_file() {
    let Some(viewer) = find_viewer() else {
        eprintln!("skipping: no native Firefox viewer binary found");
        return;
    };

    let temp = std::env::temp_dir().join(format!("burnbox-viewer-test-{}", std::process::id()));
    std::fs::create_dir_all(&temp).expect("create temp dir");
    let document = temp.join("tiny.html");
    std::fs::write(
        &document,
        "<!doctype html><title>BurnBox</title><h1>BurnBox M7 viewer test</h1>",
    )
    .expect("write tiny document");
    let screenshot = temp.join("screenshot.png");
    let _ = std::fs::remove_file(&screenshot);
    let url = format!("file://{}", document.display());

    let child = Command::new(env!("CARGO_BIN_EXE_burnbox-core"))
        .args([
            "--mode",
            "online",
            "--ram-mb",
            "256",
            "--seccomp-strict",
            "--target",
        ])
        .arg(&viewer)
        .arg("--")
        .args(["--headless", "--no-remote"])
        .arg(format!("--screenshot={}", screenshot.display()))
        .arg(&url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn burnbox-core");

    // Bound the run so a viewer regression can never hang CI.
    let pid = child.id();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let output = match rx.recv_timeout(Duration::from_secs(120)) {
        Ok(result) => result.expect("wait for burnbox-core"),
        Err(_) => {
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
            panic!("strict viewer session timed out");
        }
    };

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "strict viewer open must exit 0 (not 3/SIGSYS); stderr: {stderr}"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "stdout must be exactly two JSON lines: {stdout}"
    );
    let started: Value = serde_json::from_str(lines[0]).expect("started line is JSON");
    let wiped: Value = serde_json::from_str(lines[1]).expect("wiped line is JSON");
    assert_eq!(started["status"], "started");
    assert_eq!(wiped["status"], "wiped");
    let mount_point = started["mount_point"]
        .as_str()
        .expect("mount_point string")
        .to_string();
    assert_eq!(wiped["mount_point"], mount_point);

    assert!(
        !Path::new(&mount_point).exists(),
        "workspace leaked after viewer exit: {mount_point}"
    );
    assert!(
        screenshot.exists(),
        "viewer did not render the tiny file; stderr: {stderr}"
    );

    let leaked: Vec<String> = std::fs::read_dir("/dev/shm")
        .expect("read /dev/shm")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("burnbox-"))
        .collect();
    assert!(leaked.is_empty(), "/dev/shm workspace leak: {leaked:?}");

    let _ = std::fs::remove_dir_all(&temp);
}
