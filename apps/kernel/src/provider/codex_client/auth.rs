use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::account_profile::{
    ProviderAccountUsageAvailability, ProviderAccountUsageMeter, ProviderAccountUsageMeterKind,
    ProviderAccountUsageMeterScope, ProviderAccountUsageMeterState, ProviderAccountUsageSnapshot,
};
use crate::error::DaemonError;

use super::{CodexClient, resolve_codex_executable};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAuthStatus {
    pub provider: String,
    pub auth_state: String,
    pub account_profile: String,
    pub identity_summary: Option<String>,
    pub plan: Option<String>,
    pub login_hint: Option<String>,
    pub detected_version: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderLoginStart {
    pub provider: String,
    pub account_profile: String,
    pub login_kind: String,
    pub login_id: Option<String>,
    pub auth_url: Option<String>,
    pub verification_url: Option<String>,
    pub user_code: Option<String>,
}

impl std::fmt::Debug for ProviderLoginStart {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderLoginStart")
            .field("provider", &self.provider)
            .field("account_profile", &self.account_profile)
            .field("login_kind", &self.login_kind)
            .field("login_id", &self.login_id)
            .field("auth_url", &self.auth_url.as_ref().map(|_| "[REDACTED]"))
            .field(
                "verification_url",
                &self.verification_url.as_ref().map(|_| "[REDACTED]"),
            )
            .field("user_code", &self.user_code.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct CodexGetAccountResponse {
    account: Option<CodexAccount>,
    #[serde(rename = "requiresOpenaiAuth")]
    requires_openai_auth: bool,
    #[serde(rename = "planType", default)]
    plan_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexAccount {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexLoginStartResponse {
    #[serde(rename = "type")]
    login_kind: String,
    #[serde(rename = "loginId", default)]
    login_id: Option<String>,
    #[serde(rename = "authUrl", default)]
    auth_url: Option<String>,
    #[serde(rename = "verificationUrl", default)]
    verification_url: Option<String>,
    #[serde(rename = "userCode", default)]
    user_code: Option<String>,
}

impl CodexClient {
    pub fn auth_status(&self, account_profile: &str) -> Result<ProviderAuthStatus, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let response: CodexGetAccountResponse =
            self.send_request(&mut socket, &mut next_request_id, "account/read", json!({}))?;
        Ok(ProviderAuthStatus {
            provider: "codex".to_string(),
            auth_state: if response.account.is_some() {
                "authenticated".to_string()
            } else if response.requires_openai_auth {
                "not_logged_in".to_string()
            } else {
                "unknown".to_string()
            },
            account_profile: account_profile.to_string(),
            identity_summary: response.account.and_then(|account| account.email),
            plan: response.plan_type,
            login_hint: Some("Run /provider login codex to authenticate Codex.".to_string()),
            detected_version: codex_version().ok(),
        })
    }

    pub fn start_login(&self, account_profile: &str) -> Result<ProviderLoginStart, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let response: CodexLoginStartResponse = self.send_request(
            &mut socket,
            &mut next_request_id,
            "account/login/start",
            json!({ "type": "chatgptDeviceCode" }),
        )?;
        Ok(ProviderLoginStart {
            provider: "codex".to_string(),
            account_profile: account_profile.to_string(),
            login_kind: response.login_kind,
            login_id: response.login_id,
            auth_url: response.auth_url,
            verification_url: response.verification_url,
            user_code: response.user_code,
        })
    }

    pub fn cancel_login(&self, login_id: &str) -> Result<(), DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let _: serde_json::Value = self.send_request(
            &mut socket,
            &mut next_request_id,
            "account/login/cancel",
            json!({ "loginId": login_id }),
        )?;
        Ok(())
    }

    /// Reads every account-usage surface exposed by the official Codex app
    /// server. The methods have evolved independently, so an unavailable
    /// surface degrades the snapshot to `partial` instead of discarding meters
    /// returned by the other one.
    pub fn usage_snapshot(
        &self,
        account_profile: &str,
    ) -> Result<ProviderAccountUsageSnapshot, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let rate_limits = self
            .send_request::<serde_json::Value>(
                &mut socket,
                &mut next_request_id,
                "account/rateLimits/read",
                json!({}),
            )
            .ok();
        let usage = self
            .send_request::<serde_json::Value>(
                &mut socket,
                &mut next_request_id,
                "account/usage/read",
                json!({}),
            )
            .ok();
        Ok(normalize_codex_usage(
            account_profile,
            rate_limits.as_ref(),
            usage.as_ref(),
        ))
    }
}

fn normalize_codex_usage(
    account_profile: &str,
    rate_limits: Option<&serde_json::Value>,
    usage: Option<&serde_json::Value>,
) -> ProviderAccountUsageSnapshot {
    let observed_at_ms = crate::session::unix_epoch_ms();
    let mut meters = Vec::new();
    if let Some(value) = rate_limits {
        collect_usage_meters(value, "rate_limits", observed_at_ms, &mut meters);
    }
    if let Some(value) = usage {
        collect_usage_meters(value, "usage", observed_at_ms, &mut meters);
    }
    // Same-identity meters from a later surface are fresher, so last wins.
    let mut deduped: Vec<ProviderAccountUsageMeter> = Vec::with_capacity(meters.len());
    for meter in meters {
        match deduped
            .iter()
            .position(|existing| existing.meter_id == meter.meter_id)
        {
            Some(index) => deduped[index] = meter,
            None => deduped.push(meter),
        }
    }
    deduped.sort_by(|left, right| left.meter_id.cmp(&right.meter_id));
    let meters = deduped;
    let available_surfaces = usize::from(rate_limits.is_some()) + usize::from(usage.is_some());
    ProviderAccountUsageSnapshot {
        profile_id: account_profile.to_string(),
        provider: "codex".to_string(),
        availability: match (available_surfaces, meters.is_empty()) {
            (2, false) => ProviderAccountUsageAvailability::Available,
            (1.., false) => ProviderAccountUsageAvailability::Partial,
            (1.., true) => ProviderAccountUsageAvailability::Partial,
            _ => ProviderAccountUsageAvailability::Unavailable,
        },
        meters,
        observed_at_ms: (available_surfaces > 0).then_some(observed_at_ms),
        source: if available_surfaces > 0 {
            "codex.app_server".to_string()
        } else {
            "provider_api_unavailable".to_string()
        },
        management_url: Some("https://chatgpt.com/codex/settings/usage".to_string()),
    }
}

fn collect_usage_meters(
    value: &serde_json::Value,
    path: &str,
    observed_at_ms: u64,
    meters: &mut Vec<ProviderAccountUsageMeter>,
) {
    match value {
        serde_json::Value::Object(object) => {
            let used_percent =
                number_field(object, &["usedPercent", "used_percent"]).or_else(|| {
                    number_field(object, &["utilization"])
                        .map(|value| if value <= 1.0 { value * 100.0 } else { value })
                });
            let remaining = number_field(object, &["remaining", "balance", "credits"]);
            let used = number_field(object, &["used", "amountUsed", "amount_used"]);
            let total = number_field(object, &["total", "limit", "spendLimit", "spend_limit"]);
            if used_percent.is_some() || remaining.is_some() || used.is_some() || total.is_some() {
                let lower_path = path.to_ascii_lowercase();
                let kind = if lower_path.contains("credit") || lower_path.contains("balance") {
                    ProviderAccountUsageMeterKind::CreditBalance
                } else if lower_path.contains("spend") || lower_path.contains("cost") {
                    ProviderAccountUsageMeterKind::SpendLimit
                } else {
                    ProviderAccountUsageMeterKind::RollingLimit
                };
                let state = match used_percent {
                    Some(value) if value >= 100.0 => ProviderAccountUsageMeterState::Exhausted,
                    Some(value) if value >= 80.0 => ProviderAccountUsageMeterState::Warning,
                    Some(_) => ProviderAccountUsageMeterState::Healthy,
                    None => ProviderAccountUsageMeterState::Unknown,
                };
                let window_duration_minutes = integer_field(
                    object,
                    &[
                        "windowDurationMins",
                        "windowDurationMinutes",
                        "window_duration_minutes",
                    ],
                );
                let label = match window_duration_minutes {
                    Some(300) => "5-hour".to_string(),
                    Some(10_080) => "Weekly".to_string(),
                    Some(43_200..=44_640) => "Monthly".to_string(),
                    _ => scoped_meter_label(path),
                };
                // Meter identity is the limit kind plus window duration, not
                // the JSON traversal path. App-server payloads expose the same
                // windows under different shapes across versions and surfaces,
                // and stable identities keep the account merge from showing
                // duplicate or orphaned window meters.
                let meter_id = match kind {
                    ProviderAccountUsageMeterKind::CreditBalance => "credits".to_string(),
                    ProviderAccountUsageMeterKind::SpendLimit => "spend".to_string(),
                    _ => match window_duration_minutes {
                        Some(minutes) => format!("rolling/{minutes}"),
                        None => format!("rolling/{}", path.replace('.', "/")),
                    },
                };
                meters.push(ProviderAccountUsageMeter {
                    meter_id,
                    label,
                    kind,
                    scope: ProviderAccountUsageMeterScope::Account,
                    used_percent,
                    used,
                    remaining,
                    total,
                    unit: string_field(object, &["unit", "currency"]),
                    window_duration_minutes,
                    resets_at_ms: timestamp_field_ms(object, &["resetsAt", "resetAt", "resets_at"]),
                    state,
                    source: "codex.app_server".to_string(),
                    observed_at_ms,
                });
            }
            // Newer app-server versions return the same convenience windows
            // under `rateLimits` and the authoritative, scoped form under
            // `rateLimitsByLimitId`. Prefer the scoped form instead of showing
            // duplicate meters to the user.
            let has_scoped_rate_limits = object.keys().any(|key| {
                matches!(
                    key.as_str(),
                    "rateLimitsByLimitId" | "rate_limits_by_limit_id"
                )
            });
            for (key, child) in object {
                if has_scoped_rate_limits && matches!(key.as_str(), "rateLimits" | "rate_limits") {
                    continue;
                }
                let child_path = format!("{path}.{key}");
                collect_usage_meters(child, &child_path, observed_at_ms, meters);
            }
        }
        serde_json::Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                collect_usage_meters(child, &format!("{path}.{index}"), observed_at_ms, meters);
            }
        }
        _ => {}
    }
}

fn number_field(object: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| object.get(*key)?.as_f64())
}

fn integer_field(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<u64> {
    keys.iter().find_map(|key| {
        let value = object.get(*key)?;
        value
            .as_u64()
            .or_else(|| value.as_f64().map(|value| value as u64))
    })
}

fn string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(*key)?.as_str().map(str::to_string))
}

fn timestamp_field_ms(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<u64> {
    keys.iter().find_map(|key| {
        let value = object.get(*key)?;
        if let Some(value) = value.as_u64() {
            return Some(epoch_to_ms(value));
        }
        if let Some(value) = value.as_f64() {
            return (value > 0.0).then(|| epoch_to_ms(value as u64));
        }
        let text = value.as_str()?.trim();
        if let Ok(value) = text.parse::<u64>() {
            return Some(epoch_to_ms(value));
        }
        chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .and_then(|value| u64::try_from(value.timestamp_millis()).ok())
    })
}

/// User-facing labels must be window periods or provider-reported scoped
/// names, never positional `primary`/`secondary` family keys.
fn scoped_meter_label(path: &str) -> String {
    let readable = |segment: &str| segment.replace(['_', '-'], " ");
    let tail = path.rsplit('.').next().unwrap_or(path);
    if matches!(tail, "primary" | "secondary" | "tertiary") {
        return path
            .split('.')
            .rev()
            .nth(1)
            .map(readable)
            .unwrap_or_else(|| "rate limit".to_string());
    }
    readable(tail)
}

fn epoch_to_ms(value: u64) -> u64 {
    if value < 10_000_000_000 {
        value * 1_000
    } else {
        value
    }
}

fn codex_version() -> Result<String, DaemonError> {
    let executable = resolve_codex_executable()?;
    let output = Command::new(executable)
        .arg("--version")
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_version",
            message: error.to_string(),
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return Ok(stderr);
    }
    Err(DaemonError::LocalTransport {
        operation: "codex_version",
        message: "codex returned no version text".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::normalize_codex_usage;
    use crate::account_profile::{
        ProviderAccountUsageAvailability, ProviderAccountUsageMeterState,
    };

    #[test]
    fn normalizes_multiple_codex_limit_windows_and_credit_balance() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimits": {
                    "primary": {"usedPercent": 82.0, "windowDurationMins": 300, "resetsAt": 1_800_000_000},
                    "secondary": {"usedPercent": 100.0, "windowDurationMins": 10080}
                }
            })),
            Some(&json!({"credits": {"balance": 12.5, "unit": "USD"}})),
        );

        assert_eq!(
            snapshot.availability,
            ProviderAccountUsageAvailability::Available
        );
        assert_eq!(snapshot.meters.len(), 3);
        assert!(
            snapshot
                .meters
                .iter()
                .any(|meter| meter.state == ProviderAccountUsageMeterState::Exhausted)
        );
        assert!(
            snapshot
                .meters
                .iter()
                .any(|meter| meter.remaining == Some(12.5))
        );
    }

    #[test]
    fn prefers_scoped_codex_windows_and_uses_period_labels() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimits": {
                    "primary": {"usedPercent": 12.0, "windowDurationMins": 300},
                    "secondary": {"usedPercent": 34.0, "windowDurationMins": 10080}
                },
                "rateLimitsByLimitId": {
                    "codex": {
                        "primary": {"usedPercent": 12.0, "windowDurationMins": 300},
                        "secondary": {"usedPercent": 34.0, "windowDurationMins": 10080}
                    }
                }
            })),
            None,
        );

        assert_eq!(snapshot.meters.len(), 2);
        let five_hour = snapshot
            .meters
            .iter()
            .find(|meter| meter.window_duration_minutes == Some(300))
            .expect("5-hour window");
        let weekly = snapshot
            .meters
            .iter()
            .find(|meter| meter.window_duration_minutes == Some(10_080))
            .expect("weekly window");
        assert_eq!(five_hour.label, "5-hour");
        assert_eq!(weekly.label, "Weekly");
        assert_eq!(five_hour.meter_id, "rolling/300");
        assert_eq!(weekly.meter_id, "rolling/10080");
    }

    #[test]
    fn keeps_one_meter_per_window_across_usage_surfaces() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimits": {
                    "primary": {"usedPercent": 50.0, "windowDurationMins": 300}
                }
            })),
            Some(&json!({
                "rateLimits": {
                    "primary": {"utilization": 0.55, "windowDurationMins": 300}
                }
            })),
        );

        assert_eq!(snapshot.meters.len(), 1);
        assert_eq!(snapshot.meters[0].meter_id, "rolling/300");
        assert!(
            (snapshot.meters[0].used_percent.expect("used percentage") - 55.0).abs()
                < f64::EPSILON * 100.0
        );
    }

    #[test]
    fn keeps_distinct_durationless_windows_without_exposing_positional_labels() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimits": {
                    "primary": {"usedPercent": 40.0},
                    "secondary": {"usedPercent": 10.0}
                }
            })),
            None,
        );

        assert_eq!(snapshot.meters.len(), 2);
        assert_ne!(snapshot.meters[0].meter_id, snapshot.meters[1].meter_id);
        assert!(
            snapshot
                .meters
                .iter()
                .all(|meter| !matches!(meter.label.as_str(), "primary" | "secondary"))
        );
    }

    #[test]
    fn parses_string_and_ratio_limit_fields() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimits": {
                    "primary": {
                        "utilization": 0.4,
                        "windowDurationMins": 10080,
                        "resetsAt": "2027-01-15T12:00:00Z"
                    },
                    "secondary": {
                        "usedPercent": 10.0,
                        "resetsAt": 1_800_000_000
                    }
                }
            })),
            None,
        );

        assert_eq!(snapshot.meters.len(), 2);
        let weekly = snapshot
            .meters
            .iter()
            .find(|meter| meter.window_duration_minutes == Some(10_080))
            .expect("weekly window");
        assert_eq!(weekly.used_percent, Some(40.0));
        assert_eq!(weekly.resets_at_ms, Some(1_800_014_400_000));
        let other = snapshot
            .meters
            .iter()
            .find(|meter| meter.window_duration_minutes.is_none())
            .expect("duration-less window");
        assert_ne!(other.label, "secondary");
    }

    #[test]
    fn unavailable_codex_methods_are_explicit() {
        let snapshot = normalize_codex_usage("work", None, None);
        assert_eq!(
            snapshot.availability,
            ProviderAccountUsageAvailability::Unavailable
        );
        assert!(snapshot.meters.is_empty());
    }
}
