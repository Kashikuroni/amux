//! Integration tests driving the built amux-verify binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_amux-verify");

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Local copy of the crate's test TempDir: integration tests are a
/// separate crate and cannot see `#[cfg(test)]` items from the lib.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("amux-verify-cli-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dirs");
        }
        std::fs::write(&path, contents).expect("write file");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn verify(dir: &Path, extra: &[&str]) -> Output {
    Command::new(BIN)
        .arg("--dir")
        .arg(dir)
        .args(extra)
        .output()
        .expect("run amux-verify")
}

#[test]
fn passing_contract_exits_zero() {
    let td = TempDir::new();
    td.write(
        ".amux/verify.toml",
        "[[gate]]\nname = \"ok\"\ncmd = \"true\"\n",
    );
    let out = verify(td.path(), &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("verdict: PASSED"), "stdout: {stdout}");
    assert!(stdout.contains("[1/1] ok"), "stdout: {stdout}");
}

#[test]
fn failing_contract_exits_one_with_repro() {
    let td = TempDir::new();
    td.write(
        ".amux/verify.toml",
        "[[gate]]\nname = \"bad\"\ncmd = \"false\"\n",
    );
    let out = verify(td.path(), &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout.contains("verdict: FAILED"), "stdout: {stdout}");
    assert!(stdout.contains("repro: false"), "stdout: {stdout}");
}

#[test]
fn missing_contract_exits_two() {
    let td = TempDir::new();
    let out = verify(td.path(), &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no contract"));
}

#[test]
fn invalid_contract_exits_two_with_hint() {
    let td = TempDir::new();
    td.write(
        ".amux/verify.toml",
        "[[gate]]\nname = \"ui\"\ncmd = \"cd ui && npm test\"\n",
    );
    let out = verify(td.path(), &[]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("shell operators"), "stderr: {stderr}");
}

#[test]
fn unknown_flag_exits_two() {
    let out = Command::new(BIN)
        .arg("--frobnicate")
        .output()
        .expect("run amux-verify");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown argument"));
}

#[test]
fn help_exits_zero() {
    let out = Command::new(BIN)
        .arg("--help")
        .output()
        .expect("run amux-verify");
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("usage: amux-verify"));
}

#[test]
fn json_mode_prints_verdict_on_stdout_and_progress_on_stderr() {
    let td = TempDir::new();
    td.write(
        ".amux/verify.toml",
        "[[gate]]\nname = \"bad\"\ncmd = \"false\"\n",
    );
    let out = verify(td.path(), &["--json", "--task-id", "s1"]);
    assert_eq!(out.status.code(), Some(1));
    let verdict: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout is a JSON verdict");
    assert_eq!(verdict["passed"], false);
    assert_eq!(verdict["task_id"], "s1");
    assert_eq!(verdict["gates"][0]["status"], "failed");
    assert!(!out.stderr.is_empty(), "progress should be on stderr");
}

#[cfg(unix)]
#[test]
fn sigint_cancels_the_run_and_exits_130() {
    use std::time::{Duration, Instant};

    let td = TempDir::new();
    td.write(
        ".amux/verify.toml",
        "[[gate]]\nname = \"slow\"\ncmd = \"sleep 5\"\n",
    );
    let mut child = Command::new(BIN)
        .arg("--dir")
        .arg(td.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn amux-verify");
    std::thread::sleep(Duration::from_millis(400)); // let the gate start
                                                    // SAFETY: plain kill(2) on our own child.
    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "amux-verify did not exit after SIGINT"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    assert_eq!(status.code(), Some(130));
}
