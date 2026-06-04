//! Self-update: check GitHub Releases for a newer version, download the
//! matching macOS asset, verify its sha256, and atomically replace the
//! running binary. All network IO shells out to curl (same as usage.rs);
//! App stays IO-free — main.rs drives these via background threads.

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
