//! Claude Code subscription account info, surfaced in the header.
//!
//! Two OAuth endpoints back this, both using the same token Claude Code stores
//! (credentials file, falling back to the macOS keychain):
//!   - `GET /api/oauth/usage`   → rolling 5h / 7d / 7d-Sonnet utilization (0–100%)
//!   - `GET /api/oauth/profile` → subscription plan (`rate_limit_tier`)
//!
//! Called via `curl` to stay dependency-light and consistent with how
//! `tmux`/`git` are shelled out elsewhere. Everything is best-effort: any
//! failure yields `None` and the header simply omits that bit.

use std::process::Command;

use crate::theme as th;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

/// One usage window: utilization (0–100) and when it resets.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Window {
    pub utilization: f64,
    /// Reset time as Unix seconds (parsed from the API's `resets_at`).
    pub reset_unix: Option<i64>,
    /// Reset time pre-formatted as local "HH:MM" (filled in `fetch`, off the
    /// render path). `None` if unknown or formatting failed.
    pub reset_hhmm: Option<String>,
}

/// Latest known usage for the rolling 5-hour and 7-day windows, plus the
/// Sonnet-specific 7-day window.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    pub seven_day_sonnet: Option<Window>,
}

impl Usage {
    pub fn is_empty(&self) -> bool {
        self.five_hour.is_none() && self.seven_day.is_none() && self.seven_day_sonnet.is_none()
    }
}

/// A polled snapshot of account state: usage windows and subscription plan.
/// Each field is independent and `None` on failure, so a partial fetch still
/// updates what it can.
#[derive(Debug, Clone, Default)]
pub struct Account {
    pub usage: Option<Usage>,
    pub plan: Option<String>,
    /// Why the last usage fetch failed (short code: HTTP status like "429", or
    /// "no auth" / "net" / "parse"), or `None` if it succeeded. Surfaced in the
    /// header so a blank limits area is explainable.
    pub usage_error: Option<String>,
}

fn pct_color(p: i64) -> Color {
    if p >= 80 {
        Color::Red
    } else if p >= 50 {
        Color::Indexed(208) // orange
    } else {
        Color::Green
    }
}

pub(crate) fn limit_spans(
    label: &str,
    window: Option<&Window>,
    show_reset: bool,
) -> Vec<Span<'static>> {
    let Some(w) = window else {
        return Vec::new();
    };
    let p = w.utilization.round() as i64;
    let value_style = Style::default()
        .fg(pct_color(p))
        .add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::styled(format!("{label} "), Style::default().fg(th::MUTED)),
        Span::styled(format!("{p}%"), value_style),
    ];
    if show_reset {
        if let Some(t) = &w.reset_hhmm {
            spans.push(Span::styled(
                format!(" resets {t}"),
                Style::default().fg(th::DIM),
            ));
        }
    }
    spans
}

pub(crate) fn account_right_spans(
    usage: Option<&Usage>,
    plan: Option<&str>,
    usage_error: Option<&str>,
) -> Vec<Span<'static>> {
    let mut out: Vec<Span> = Vec::new();

    if let Some(p) = plan {
        out.push(Span::styled(
            p.to_string(),
            Style::default()
                .fg(th::TEXT_BOLD)
                .add_modifier(Modifier::BOLD),
        ));
    }

    if let Some(u) = usage {
        let blocks: Vec<Vec<Span>> = [
            limit_spans("5h", u.five_hour.as_ref(), true),
            limit_spans("7d", u.seven_day.as_ref(), false),
            limit_spans("sonnet", u.seven_day_sonnet.as_ref(), false),
        ]
        .into_iter()
        .filter(|b| !b.is_empty())
        .collect();

        for (i, block) in blocks.into_iter().enumerate() {
            if i > 0 || !out.is_empty() {
                out.push(Span::styled(
                    format!(" {} ", th::SEP),
                    Style::default().fg(th::DIM),
                ));
            }
            out.extend(block);
        }
    } else if let Some(err) = usage_error {
        if !out.is_empty() {
            out.push(Span::styled(
                format!(" {} ", th::SEP),
                Style::default().fg(th::DIM),
            ));
        }
        out.push(Span::styled(
            format!("limits ✗ {err}"),
            Style::default().fg(th::DIM),
        ));
    }

    out
}

/// Fetches usage + plan in one cycle (called from the background poller).
pub fn fetch_account() -> Account {
    let (usage, usage_error) = match fetch_usage() {
        Ok(u) => (Some(u), None),
        Err(e) => (None, Some(e)),
    };
    Account {
        usage,
        plan: fetch_plan(),
        usage_error,
    }
}

/// GETs an OAuth endpoint with the stored token. On success returns the body;
/// on failure a short reason: "no auth" (no token), "net" (curl/connection
/// failure), or the HTTP status code ("401", "429", …) for a non-2xx response.
/// `curl` is run without `-f`, so HTTP errors arrive as a 0 exit with an error
/// body — we read the status via `-w` to distinguish them.
fn oauth_get(path: &str) -> Result<Vec<u8>, String> {
    let token = read_token().ok_or("no auth")?;
    let out = Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "10",
            &format!("https://api.anthropic.com{path}"),
            "-H",
            &format!("Authorization: Bearer {token}"),
            "-H",
            "anthropic-beta: oauth-2025-04-20",
            // Append "\n<status>" to the body so we can read the HTTP code.
            "-w",
            "\n%{http_code}",
        ])
        .output()
        .map_err(|_| "net".to_string())?;
    if !out.status.success() {
        return Err("net".to_string());
    }
    // Split the trailing "\n<status>" the -w format appended.
    let mut body = out.stdout;
    let Some(nl) = body.iter().rposition(|&b| b == b'\n') else {
        return Err("net".to_string());
    };
    let code: u16 = String::from_utf8_lossy(&body[nl + 1..])
        .trim()
        .parse()
        .unwrap_or(0);
    body.truncate(nl);
    if (200..300).contains(&code) {
        Ok(body)
    } else {
        Err(match code {
            401 => "token expired".to_string(),
            _ => code.to_string(),
        })
    }
}

/// Fetches current usage. Returns `Err(reason)` when unauthenticated, offline,
/// rate-limited, or the token has expired — callers keep the last good value and
/// can surface the reason.
fn fetch_usage() -> Result<Usage, String> {
    let body = oauth_get("/api/oauth/usage")?;
    let mut usage = parse_usage(&body).ok_or("parse")?;
    // Format reset times to local HH:MM here (off the render path).
    for w in [
        &mut usage.five_hour,
        &mut usage.seven_day,
        &mut usage.seven_day_sonnet,
    ]
    .into_iter()
    .flatten()
    {
        if let Some(t) = w.reset_unix {
            let s = crate::timeutil::local_hhmm(t);
            if !s.is_empty() {
                w.reset_hhmm = Some(s);
            }
        }
    }
    Ok(usage)
}

/// Fetches the subscription plan label (e.g. "Max 5×") from the profile.
fn fetch_plan() -> Option<String> {
    parse_plan(&oauth_get("/api/oauth/profile").ok()?)
}

/// Parses the `/api/oauth/usage` JSON body. Split out so it can be unit-tested
/// without a network round-trip. Does not format reset times (see `fetch_usage`).
fn parse_usage(body: &[u8]) -> Option<Usage> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let window = |key: &str| -> Option<Window> {
        let w = v.get(key).filter(|w| !w.is_null())?;
        let utilization = w.get("utilization")?.as_f64()?;
        let reset_unix = w
            .get("resets_at")
            .and_then(|x| x.as_str())
            .and_then(crate::timeutil::parse_iso8601);
        Some(Window {
            utilization,
            reset_unix,
            reset_hhmm: None,
        })
    };
    let usage = Usage {
        five_hour: window("five_hour"),
        seven_day: window("seven_day"),
        seven_day_sonnet: window("seven_day_sonnet"),
    };
    // A 200 with all windows null (e.g. an API-key account with no subscription
    // limits) is not worth showing.
    if usage.is_empty() {
        None
    } else {
        Some(usage)
    }
}

/// Reads the Claude Code OAuth access token: the credentials file first (Linux /
/// custom setups), then the macOS keychain entry Claude Code writes.
fn read_token() -> Option<String> {
    if let Some(home) = std::env::var_os("HOME") {
        let path = std::path::Path::new(&home).join(".claude/.credentials.json");
        if let Ok(s) = std::fs::read_to_string(&path) {
            if let Some(t) = token_from_json(&s) {
                return Some(t);
            }
        }
    }
    let out = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    token_from_json(&String::from_utf8_lossy(&out.stdout))
}

fn token_from_json(s: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    v.get("claudeAiOauth")?
        .get("accessToken")?
        .as_str()
        .map(str::to_string)
}

/// Extracts and humanizes the plan from a `/api/oauth/profile` body, reading
/// `organization.rate_limit_tier` (e.g. "default_claude_max_5x" → "Max 5×").
fn parse_plan(body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let tier = v.get("organization")?.get("rate_limit_tier")?.as_str()?;
    Some(plan_label(tier))
}

/// Turns a raw `rate_limit_tier` slug into a short badge: base plan + any `Nx`
/// multiplier (e.g. "default_claude_max_5x" → "Max 5×", "claude_pro" → "Pro").
fn plan_label(tier: &str) -> String {
    let t = tier.to_lowercase();
    let base = if t.contains("max") {
        "Max"
    } else if t.contains("pro") {
        "Pro"
    } else if t.contains("team") {
        "Team"
    } else if t.contains("enterprise") {
        "Enterprise"
    } else {
        "Claude"
    };
    // A trailing "_<n>x" segment is the rate multiplier.
    let mult = t.rsplit('_').find_map(|seg| {
        let digits = seg.strip_suffix('x')?;
        (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
            .then(|| digits.to_string())
    });
    match mult {
        Some(n) => format!("{base} {n}×"),
        None => base.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_shape() {
        let body = br#"{
            "five_hour": {"utilization": 77.0, "resets_at": "2026-06-02T10:40:01Z"},
            "seven_day": {"utilization": 8.0, "resets_at": "2026-06-06T04:00:01Z"},
            "seven_day_sonnet": {"utilization": 1.0, "resets_at": "2026-06-06T04:00:01Z"},
            "seven_day_opus": null,
            "extra_usage": {"is_enabled": false}
        }"#;
        let u = parse_usage(body).expect("should parse");
        let five = u.five_hour.expect("five_hour present");
        assert_eq!(five.utilization, 77.0);
        assert_eq!(
            five.reset_unix,
            crate::timeutil::parse_iso8601("2026-06-02T10:40:01Z")
        );
        assert!(five.reset_unix.is_some());
        assert_eq!(u.seven_day.expect("seven_day present").utilization, 8.0);
        assert_eq!(u.seven_day_sonnet.expect("sonnet present").utilization, 1.0);
    }

    #[test]
    fn all_null_windows_yield_none() {
        let body = br#"{"five_hour": null, "seven_day": null, "seven_day_sonnet": null}"#;
        assert!(parse_usage(body).is_none());
    }

    #[test]
    fn garbage_yields_none() {
        assert!(parse_usage(b"not json").is_none());
    }

    #[test]
    fn extracts_token_from_credentials_json() {
        let s = r#"{"claudeAiOauth": {"accessToken": "sk-abc", "refreshToken": "x"}}"#;
        assert_eq!(token_from_json(s), Some("sk-abc".to_string()));
        assert_eq!(token_from_json("{}"), None);
    }

    #[test]
    fn plan_label_maps_tiers() {
        assert_eq!(plan_label("default_claude_max_5x"), "Max 5×");
        assert_eq!(plan_label("default_claude_max_20x"), "Max 20×");
        assert_eq!(plan_label("claude_pro"), "Pro");
        assert_eq!(plan_label("something_else"), "Claude");
    }

    #[test]
    fn parses_plan_from_profile() {
        let body = br#"{"organization": {"rate_limit_tier": "default_claude_max_5x"}}"#;
        assert_eq!(parse_plan(body), Some("Max 5×".to_string()));
        assert_eq!(parse_plan(b"{}"), None);
    }

    #[test]
    fn limit_spans_renders_label_pct_and_reset() {
        let w = Window {
            utilization: 77.0,
            reset_unix: None,
            reset_hhmm: Some("14:40".to_string()),
        };
        let spans = limit_spans("5h", Some(&w), true);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("5h"), "missing label: {text}");
        assert!(text.contains("77%"), "missing pct: {text}");
        assert!(text.contains("resets 14:40"), "missing reset: {text}");
    }

    #[test]
    fn limit_spans_omits_reset_when_show_reset_false() {
        let w = Window {
            utilization: 8.0,
            reset_unix: None,
            reset_hhmm: Some("04:00".to_string()),
        };
        let spans = limit_spans("7d", Some(&w), false);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("7d"), "missing label: {text}");
        assert!(text.contains("8%"), "missing pct: {text}");
        assert!(!text.contains("resets"), "must not show reset: {text}");
    }

    #[test]
    fn limit_spans_empty_when_window_absent() {
        assert!(limit_spans("5h", None, true).is_empty());
    }

    #[test]
    fn pct_color_thresholds() {
        use ratatui::style::Color;
        assert_eq!(pct_color(0), Color::Green);
        assert_eq!(pct_color(49), Color::Green);
        assert_eq!(pct_color(50), Color::Indexed(208));
        assert_eq!(pct_color(79), Color::Indexed(208));
        assert_eq!(pct_color(80), Color::Red);
        assert_eq!(pct_color(100), Color::Red);
    }

    #[test]
    fn account_right_spans_empty_when_no_data() {
        assert!(account_right_spans(None, None, None).is_empty());
    }

    #[test]
    fn account_right_spans_includes_plan_and_usage() {
        let usage = Usage {
            five_hour: Some(Window {
                utilization: 77.0,
                reset_unix: None,
                reset_hhmm: Some("14:40".to_string()),
            }),
            seven_day: Some(Window {
                utilization: 8.0,
                reset_unix: None,
                reset_hhmm: None,
            }),
            seven_day_sonnet: None,
        };
        let spans = account_right_spans(Some(&usage), Some("Max 5×"), None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Max 5×"), "missing plan: {text}");
        assert!(text.contains("5h"), "missing 5h label: {text}");
        assert!(text.contains("77%"), "missing 5h pct: {text}");
        assert!(text.contains("7d"), "missing 7d label: {text}");
        assert!(text.contains("8%"), "missing 7d pct: {text}");
    }

    #[test]
    fn account_right_spans_shows_error_when_no_usage() {
        let spans = account_right_spans(None, None, Some("429"));
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("limits"), "missing 'limits': {text}");
        assert!(text.contains("429"), "missing error code: {text}");
    }

    #[test]
    fn account_right_spans_plan_with_error() {
        let spans = account_right_spans(None, Some("Pro"), Some("net"));
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Pro"), "missing plan: {text}");
        assert!(text.contains("net"), "missing error: {text}");
    }
}
