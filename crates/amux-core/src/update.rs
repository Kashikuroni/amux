//! Self-update: check GitHub Releases for a newer version, download the
//! matching macOS asset, verify its sha256, and atomically replace the
//! running binary. All network IO shells out to curl (same as usage.rs);
//! App stays IO-free — main.rs drives these via background threads.

use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

const REPO: &str = "Kashikuroni/amux";

/// Shown when self-update cannot proceed (no write access etc.).
pub const MANUAL_HINT: &str =
    "update manually: cargo binstall --git https://github.com/Kashikuroni/amux amux";

/// A newer release found on GitHub.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateInfo {
    /// Bare version, e.g. "0.3.0".
    pub version: String,
    /// Asset URL for this machine's target triple.
    pub url: String,
}

/// Install progress reported by the background installer thread.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStage {
    Downloading,
    Verifying,
    Installing,
    /// Installed; holds the new version. Restart to run it.
    Done(String),
    /// Human-readable reason; the running binary is untouched.
    Failed(String),
}

/// `vX.Y.Z` → (X, Y, Z). Anything else (pre-releases, garbage) → None.
pub fn parse_tag(tag: &str) -> Option<(u32, u32, u32)> {
    let t = tag.strip_prefix('v')?;
    let mut it = t.split('.');
    let maj = it.next()?.parse().ok()?;
    let min = it.next()?.parse().ok()?;
    let pat = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((maj, min, pat))
}

/// True when the `vX.Y.Z` tag is strictly newer than the local "X.Y.Z".
/// `local` must be bare (no 'v' prefix) — a prefixed value yields a silent false.
pub fn is_newer(remote_tag: &str, local: &str) -> bool {
    match (parse_tag(remote_tag), parse_tag(&format!("v{local}"))) {
        (Some(r), Some(l)) => r > l,
        _ => false,
    }
}

/// Target triple matching the release asset naming (CI builds macOS only).
pub fn release_target() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64-apple-darwin"
    } else {
        "x86_64-apple-darwin"
    }
}

/// Asset URL by the CI naming convention (see .github/workflows/release.yml).
pub fn asset_url(tag: &str) -> String {
    format!(
        "https://github.com/{REPO}/releases/download/{tag}/amux-{tag}-{}.tar.gz",
        release_target()
    )
}

/// Parses a `releases/latest` body. Some only when the release is strictly
/// newer than `local`; any parse problem → None (the check stays silent).
pub fn parse_latest_release(body: &str, local: &str) -> Option<UpdateInfo> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let tag = v.get("tag_name")?.as_str()?;
    if !is_newer(tag, local) {
        return None;
    }
    Some(UpdateInfo {
        version: tag.trim_start_matches('v').to_string(),
        url: asset_url(tag),
    })
}

/// One-shot background release check (the usage-poller pattern). Sends at most
/// one UpdateInfo; any failure — net, parse, already current — ends the thread
/// silently. Disabled in debug builds (dev runs from target/).
pub fn spawn_check() -> mpsc::Receiver<UpdateInfo> {
    let (tx, rx) = mpsc::channel();
    if cfg!(debug_assertions) {
        return rx;
    }
    thread::spawn(move || {
        let Ok(out) = Command::new("curl")
            .args([
                "-sS",
                "--max-time",
                "10",
                &format!("https://api.github.com/repos/{REPO}/releases/latest"),
                "-H",
                "User-Agent: amux",
            ])
            .output()
        else {
            return;
        };
        if !out.status.success() {
            return;
        }
        let body = String::from_utf8_lossy(&out.stdout);
        if let Some(info) = parse_latest_release(&body, env!("CARGO_PKG_VERSION")) {
            let _ = tx.send(info);
        }
    });
    rx
}

/// Atomically replaces `dest` with `new_bin`: stage a copy next to the
/// destination (same volume, so rename can't cross devices), swap via rename,
/// drop the old file. `dest` is untouched until the final rename; if that
/// fails the original is put back.
pub fn swap_binary(new_bin: &Path, dest: &Path) -> Result<(), String> {
    let dir = dest.parent().ok_or("destination has no parent dir")?;
    let staged = dir.join("amux.new");
    let old = dir.join("amux.old");
    std::fs::copy(new_bin, &staged).map_err(|e| format!("copy: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod: {e}"))?;
    }
    std::fs::rename(dest, &old).map_err(|e| format!("rename old: {e}"))?;
    if let Err(e) = std::fs::rename(&staged, dest) {
        // Put the original back: the update failed but amux still works.
        let _ = std::fs::rename(&old, dest);
        return Err(format!("rename new: {e}"));
    }
    let _ = std::fs::remove_file(&old);
    Ok(())
}

/// `curl -f`: non-2xx → nonzero exit (detects a missing .sha256 asset);
/// `-L` follows GitHub's redirect to the CDN.
fn curl_to(url: &str, dest: &Path) -> bool {
    Command::new("curl")
        .args(["-sSfL", "--max-time", "120", "-o"])
        .arg(dest)
        .arg(url)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Compares the file's sha256 against the first token of the `.sha256` file.
fn sha256_matches(file: &Path, sha_file: &Path) -> Result<bool, String> {
    let expected = std::fs::read_to_string(sha_file).map_err(|e| format!("read checksum: {e}"))?;
    let expected = expected
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();
    if expected.len() != 64 {
        return Err("malformed checksum file".into());
    }
    let out = Command::new("shasum")
        .args(["-a", "256"])
        .arg(file)
        .output()
        .map_err(|e| format!("shasum: {e}"))?;
    let actual = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();
    Ok(actual == expected)
}

/// Downloads, verifies and installs `info` in a background thread, streaming
/// progress. The running binary is only touched in the final swap; every
/// failure path leaves it as-is and removes the temp dir.
pub fn spawn_install(info: UpdateInfo) -> mpsc::Receiver<UpdateStage> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let send = |s: UpdateStage| {
            let _ = tx.send(s);
        };
        let Ok(exe) = std::env::current_exe() else {
            send(UpdateStage::Failed(format!(
                "cannot locate the running binary — {MANUAL_HINT}"
            )));
            return;
        };
        let Some(exe_dir) = exe.parent().map(Path::to_path_buf) else {
            send(UpdateStage::Failed(format!(
                "binary has no parent dir — {MANUAL_HINT}"
            )));
            return;
        };
        // Writability probe up front, before downloading anything.
        let probe = exe_dir.join(".amux.update.probe");
        if std::fs::write(&probe, b"").is_err() {
            send(UpdateStage::Failed(format!(
                "no write access to {} — {MANUAL_HINT}",
                exe_dir.display()
            )));
            return;
        }
        let _ = std::fs::remove_file(&probe);

        let work = std::env::temp_dir().join(format!("amux-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        if std::fs::create_dir_all(&work).is_err() {
            send(UpdateStage::Failed("cannot create temp dir".into()));
            return;
        }
        let fail = |reason: String| {
            let _ = std::fs::remove_dir_all(&work);
            let _ = tx.send(UpdateStage::Failed(reason));
        };

        send(UpdateStage::Downloading);
        let tgz = work.join("amux.tar.gz");
        if !curl_to(&info.url, &tgz) {
            fail("download failed".into());
            return;
        }

        send(UpdateStage::Verifying);
        let sha = work.join("amux.tar.gz.sha256");
        if curl_to(&format!("{}.sha256", info.url), &sha) {
            match sha256_matches(&tgz, &sha) {
                Ok(true) => {}
                Ok(false) => {
                    fail("checksum mismatch".into());
                    return;
                }
                Err(e) => {
                    fail(e);
                    return;
                }
            }
        }
        // No .sha256 asset (pre-checksum release) → skip verification.

        send(UpdateStage::Installing);
        let untar = Command::new("tar")
            .arg("-xzf")
            .arg(&tgz)
            .arg("-C")
            .arg(&work)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let new_bin = work.join("amux");
        if !untar || !new_bin.exists() {
            fail("archive did not contain amux".into());
            return;
        }
        match swap_binary(&new_bin, &exe) {
            Ok(()) => send(UpdateStage::Done(info.version.clone())),
            Err(e) => send(UpdateStage::Failed(e)),
        }
        let _ = std::fs::remove_dir_all(&work);
    });
    rx
}

/// Replaces this process with the (just-updated) binary at the same path.
/// Only returns on failure. tmux sessions live in the tmux server and are
/// unaffected.
pub fn restart() -> std::io::Error {
    use std::os::unix::process::CommandExt;
    match std::env::current_exe() {
        Ok(exe) => Command::new(exe).exec(),
        Err(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tag_accepts_semver_rejects_garbage() {
        assert_eq!(parse_tag("v0.3.0"), Some((0, 3, 0)));
        assert_eq!(parse_tag("v12.0.5"), Some((12, 0, 5)));
        assert_eq!(parse_tag("0.3.0"), None); // no v prefix
        assert_eq!(parse_tag("v0.3"), None); // too short
        assert_eq!(parse_tag("v0.3.0.1"), None); // too long
        assert_eq!(parse_tag("v0.3.0-rc1"), None); // pre-release
        assert_eq!(parse_tag("latest"), None);
        assert_eq!(parse_tag(""), None);
    }

    #[test]
    fn is_newer_strictly_compares() {
        assert!(is_newer("v0.3.0", "0.2.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("v0.2.0", "0.2.0")); // equal
        assert!(!is_newer("v0.1.9", "0.2.0")); // older
        assert!(!is_newer("garbage", "0.2.0"));
        assert!(!is_newer("v0.3.0", "garbage"));
    }

    #[test]
    fn asset_url_matches_ci_naming() {
        let url = asset_url("v0.3.0");
        assert!(url.starts_with(
            "https://github.com/Kashikuroni/amux/releases/download/v0.3.0/amux-v0.3.0-"
        ));
        assert!(url.ends_with("-apple-darwin.tar.gz"));
    }

    #[test]
    fn swap_binary_replaces_dest_and_cleans_up() {
        let dir = std::env::temp_dir().join(format!("amux_swap_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("amux");
        let new_bin = dir.join("downloaded");
        std::fs::write(&dest, b"old").unwrap();
        std::fs::write(&new_bin, b"new").unwrap();

        swap_binary(&new_bin, &dest).expect("swap should succeed");

        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert!(!dir.join("amux.old").exists(), "old binary cleaned up");
        assert!(!dir.join("amux.new").exists(), "staging cleaned up");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "must be executable");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_latest_release_only_offers_newer() {
        let body = |tag: &str| format!(r#"{{"tag_name": "{tag}", "name": "amux {tag}"}}"#);
        let info = parse_latest_release(&body("v0.3.0"), "0.2.0").expect("newer offered");
        assert_eq!(info.version, "0.3.0");
        assert!(info.url.contains("/v0.3.0/"));
        assert!(parse_latest_release(&body("v0.2.0"), "0.2.0").is_none()); // same
        assert!(parse_latest_release(&body("v0.1.0"), "0.2.0").is_none()); // older
        assert!(parse_latest_release("not json", "0.2.0").is_none()); // silence
        assert!(parse_latest_release(r#"{"message": "rate limited"}"#, "0.2.0").is_none());
    }
}
