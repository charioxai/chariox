//! Claude's account usage windows (5-hour and weekly percent used, with reset
//! times), read the way Claude Code reads them for `/usage`: `GET
//! /api/oauth/usage` with the account's OAuth bearer token and the
//! `oauth-2025-04-20` beta header. The `get_usage` control request of the
//! official CLI returns the same windows for logins the CLI manages itself.
use std::io::Read;
use std::time::Duration;

use serde_json::Value;

use crate::account_profile::ProviderAccountUsageSnapshot;
use crate::provider::ProviderUsageReadError;

const CLAUDE_USAGE_API_BASE: &str = "https://api.anthropic.com";
const CLAUDE_USAGE_PATH: &str = "/api/oauth/usage";
const CLAUDE_OAUTH_BETA: &str = "oauth-2025-04-20";
const CLAUDE_USAGE_MAX_BODY_BYTES: u64 = 256 * 1024;
const CLAUDE_USAGE_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The endpoint refuses a token for an hour after too many reads; a longer
/// advertised wait is not trusted beyond the next day.
const CLAUDE_USAGE_MAX_RETRY_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// A credential without the `user:profile` scope can never read usage.
const CLAUDE_USAGE_SCOPE_RETRY: Duration = Duration::from_secs(6 * 60 * 60);

/// Reads the account's usage windows with an enrolled OAuth token.
pub(crate) fn read_claude_oauth_usage(
    token: &str,
    account_profile: &str,
) -> Result<ProviderAccountUsageSnapshot, ProviderUsageReadError> {
    read_claude_oauth_usage_from(CLAUDE_USAGE_API_BASE, token, account_profile)
}

fn read_claude_oauth_usage_from(
    base_url: &str,
    token: &str,
    account_profile: &str,
) -> Result<ProviderAccountUsageSnapshot, ProviderUsageReadError> {
    let response = crate::outbound_dial::agent()
        .get(&format!("{base_url}{CLAUDE_USAGE_PATH}"))
        .timeout(CLAUDE_USAGE_REQUEST_TIMEOUT)
        .set("Authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", CLAUDE_OAUTH_BETA)
        .set("Content-Type", "application/json")
        .set("User-Agent", "claude-code/chariox-usage")
        .call();
    let response = match response {
        Ok(response) => response,
        Err(ureq::Error::Status(429, response)) => {
            let wait = response
                .header("retry-after")
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(Duration::from_secs)
                .map(|wait| wait.min(CLAUDE_USAGE_MAX_RETRY_AFTER));
            return Err(
                ProviderUsageReadError::new("Claude usage read was rate limited").retry_after(wait),
            );
        }
        Err(ureq::Error::Status(403, _)) => {
            return Err(ProviderUsageReadError::new(
                "Claude credential lacks the user:profile scope needed to read usage",
            )
            .retry_after(Some(CLAUDE_USAGE_SCOPE_RETRY)));
        }
        Err(ureq::Error::Status(status, _)) => {
            return Err(ProviderUsageReadError::new(format!(
                "Claude usage read answered HTTP {status}"
            )));
        }
        Err(ureq::Error::Transport(error)) => {
            return Err(ProviderUsageReadError::new(format!(
                "Claude usage read failed: {}",
                error.kind()
            )));
        }
    };
    let mut text = String::new();
    response
        .into_reader()
        .take(CLAUDE_USAGE_MAX_BODY_BYTES)
        .read_to_string(&mut text)
        .map_err(|_| ProviderUsageReadError::new("Claude usage read could not be read"))?;
    let body: Value = serde_json::from_str(&text)
        .map_err(|_| ProviderUsageReadError::new("Claude usage read returned invalid JSON"))?;
    claude_usage_snapshot_from_windows(&body, account_profile)
}

/// Builds the snapshot from an object holding `five_hour` / `seven_day`
/// windows of `{utilization: 0-100, resets_at}`. A window of another shape is
/// left out, so a changed response never turns into a made-up number.
pub(crate) fn claude_usage_snapshot_from_windows(
    windows: &Value,
    account_profile: &str,
) -> Result<ProviderAccountUsageSnapshot, ProviderUsageReadError> {
    let mut rate_limits = serde_json::Map::new();
    for window in ["five_hour", "seven_day"] {
        let Some(value) = windows.get(window).filter(|value| value.is_object()) else {
            continue;
        };
        let Some(percent) = value
            .get("utilization")
            .and_then(Value::as_f64)
            .filter(|percent| percent.is_finite() && (0.0..=100.0).contains(percent))
        else {
            continue;
        };
        rate_limits.insert(
            window.to_string(),
            serde_json::json!({
                "used_percentage": percent,
                "resets_at": value.get("resets_at").cloned().unwrap_or(Value::Null),
            }),
        );
    }
    let mut usage = crate::provider::claude_status_line_usage_snapshot(
        &serde_json::json!({ "rate_limits": rate_limits }),
    )
    .ok_or_else(|| {
        ProviderUsageReadError::new("Claude did not return 5-hour or weekly usage windows")
    })?;
    usage.profile_id = account_profile.to_string();
    Ok(usage)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    fn serve_once(status_line: &str, headers: &str, body: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status_line}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]).to_lowercase();
            assert!(request.starts_with("get /api/oauth/usage "));
            assert!(request.contains("authorization: bearer test-token"));
            assert!(request.contains("anthropic-beta: oauth-2025-04-20"));
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{address}")
    }

    #[test]
    fn reads_both_windows() {
        let base = serve_once(
            "200 OK",
            "Content-Type: application/json\r\n",
            r#"{"five_hour":{"utilization":12.5,"resets_at":"2099-01-01T05:00:00+00:00"},
                "seven_day":{"utilization":5.0,"resets_at":"2099-01-07T00:00:00+00:00"},
                "seven_day_opus":null}"#,
        );
        let usage = read_claude_oauth_usage_from(&base, "test-token", "profile-a").unwrap();
        assert_eq!(usage.profile_id, "profile-a");
        let ids: Vec<_> = usage.meters.iter().map(|m| m.meter_id.as_str()).collect();
        assert_eq!(ids, ["rate_limit/five_hour", "rate_limit/seven_day"]);
        assert_eq!(usage.meters[0].used_percent, Some(12.5));
        assert_eq!(usage.meters[1].used_percent, Some(5.0));
        assert!(usage.meters[0]
            .resets_at_ms
            .is_some_and(|ms| ms > 4_000_000_000_000));
    }

    #[test]
    fn rate_limit_carries_the_advertised_wait() {
        let base = serve_once("429 Too Many Requests", "Retry-After: 3596\r\n", "{}");
        let error = read_claude_oauth_usage_from(&base, "test-token", "profile-a").unwrap_err();
        assert_eq!(error.retry_after, Some(Duration::from_secs(3596)));
    }

    #[test]
    fn missing_profile_scope_is_a_long_wait_not_a_retry_storm() {
        let base = serve_once("403 Forbidden", "", "{}");
        let error = read_claude_oauth_usage_from(&base, "test-token", "profile-a").unwrap_err();
        assert_eq!(error.retry_after, Some(CLAUDE_USAGE_SCOPE_RETRY));
    }

    #[test]
    fn windows_of_another_shape_are_left_out_never_guessed() {
        let usage = claude_usage_snapshot_from_windows(
            &serde_json::json!({
                "five_hour": {"utilization": "high", "resets_at": null},
                "seven_day": {"utilization": 140.0, "resets_at": null},
            }),
            "p",
        );
        assert!(usage.is_err());
        let usage = claude_usage_snapshot_from_windows(
            &serde_json::json!({
                "five_hour": {"utilization": 0.0, "resets_at": null},
                "seven_day": {"percent": 3},
            }),
            "p",
        )
        .unwrap();
        assert_eq!(usage.meters.len(), 1);
        assert_eq!(usage.meters[0].used_percent, Some(0.0));
    }
}
