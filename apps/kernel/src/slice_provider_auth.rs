use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How long a home-directory provider-auth scan is reused before re-reading
/// the auth files. `relay_registration` calls the scan on every registration
/// and peer request while holding the app lock, so caching keeps that path
/// off the disk without meaningfully staling login state.
const HOME_PROVIDER_AUTH_CACHE_TTL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceProviderAuthState {
    Unknown,
    NotConfigured,
    Configured,
    Authenticated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceProviderAuthSummary {
    pub provider: String,
    pub state: SliceProviderAuthState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub source: String,
}

impl SliceProviderAuthSummary {
    pub fn alias_or_identity(&self) -> Option<&str> {
        self.alias
            .as_deref()
            .or(self.email.as_deref())
            .or(self.account_id.as_deref())
    }
}

pub fn inspect_home_provider_auth(home_dir: &Path) -> Vec<SliceProviderAuthSummary> {
    let cache_key = home_dir.to_string_lossy().to_string();
    if let Some(cached) = cached_home_provider_auth(&cache_key) {
        return cached;
    }
    let summaries = read_home_provider_auth(home_dir);
    store_home_provider_auth(cache_key, &summaries);
    summaries
}

/// Uncached read of the home-directory provider-auth files.
pub fn read_home_provider_auth(home_dir: &Path) -> Vec<SliceProviderAuthSummary> {
    let mut summaries = Vec::new();
    if let Ok(text) = std::fs::read_to_string(home_dir.join(".codex").join("auth.json")) {
        if let Some(summary) = parse_codex_auth_json(&text) {
            summaries.push(summary);
        }
    }
    if let Ok(text) = std::fs::read_to_string(
        home_dir
            .join(".local")
            .join("share")
            .join("opencode")
            .join("auth.json"),
    ) {
        summaries.extend(parse_opencode_auth_json(&text));
    }
    if let Ok(text) = std::fs::read_to_string(home_dir.join(".claude.json")) {
        if let Some(summary) = parse_claude_settings_json(&text) {
            summaries.push(summary);
        }
    }
    if let Ok(text) = std::fs::read_to_string(home_dir.join(".claude").join("stats-cache.json")) {
        if let Some(summary) = parse_claude_status_cache_json(&text) {
            summaries.push(summary);
        }
    }
    merge_provider_auth_summaries(summaries)
}

struct HomeProviderAuthCacheEntry {
    read_at: Instant,
    summaries: Vec<SliceProviderAuthSummary>,
}

fn home_provider_auth_cache() -> &'static Mutex<HashMap<String, HomeProviderAuthCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, HomeProviderAuthCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_home_provider_auth(cache_key: &str) -> Option<Vec<SliceProviderAuthSummary>> {
    let cache = home_provider_auth_cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let entry = cache.get(cache_key)?;
    (entry.read_at.elapsed() < HOME_PROVIDER_AUTH_CACHE_TTL).then(|| entry.summaries.clone())
}

fn store_home_provider_auth(cache_key: String, summaries: &[SliceProviderAuthSummary]) {
    home_provider_auth_cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(
            cache_key,
            HomeProviderAuthCacheEntry {
                read_at: Instant::now(),
                summaries: summaries.to_vec(),
            },
        );
}

/// Drop cached home-directory provider-auth scans so the next inspection
/// re-reads the auth files (used when auth is known to have just changed).
pub fn clear_home_provider_auth_cache() {
    home_provider_auth_cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

pub fn parse_codex_auth_json(text: &str) -> Option<SliceProviderAuthSummary> {
    let value: Value = serde_json::from_str(text).ok()?;
    let account_id = value
        .pointer("/tokens/account_id")
        .and_then(Value::as_str)
        .or_else(|| value.get("account_id").and_then(Value::as_str))
        .map(str::to_string);
    let auth_type = value
        .get("auth_mode")
        .and_then(Value::as_str)
        .map(str::to_string);
    if account_id.is_none() && auth_type.is_none() && value.get("OPENAI_API_KEY").is_none() {
        return None;
    }
    Some(SliceProviderAuthSummary {
        provider: "codex".to_string(),
        state: SliceProviderAuthState::Configured,
        auth_type,
        account_id,
        email: None,
        organization_id: None,
        organization_name: None,
        subscription_type: None,
        alias: None,
        source: "home_codex_auth_json".to_string(),
    })
}

pub fn parse_opencode_auth_json(text: &str) -> Vec<SliceProviderAuthSummary> {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    object
        .iter()
        .filter_map(|(provider, config)| {
            let auth_type = config
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string);
            let account_id = config
                .get("accountId")
                .or_else(|| config.get("account_id"))
                .and_then(Value::as_str)
                .map(str::to_string);
            if auth_type.is_none() && account_id.is_none() {
                return None;
            }
            Some(SliceProviderAuthSummary {
                provider: format!("opencode:{provider}"),
                state: SliceProviderAuthState::Configured,
                auth_type,
                account_id,
                email: None,
                organization_id: None,
                organization_name: None,
                subscription_type: None,
                alias: None,
                source: "home_opencode_auth_json".to_string(),
            })
        })
        .collect()
}

pub fn parse_claude_status_json(text: &str) -> Option<SliceProviderAuthSummary> {
    let value: Value = serde_json::from_str(text).ok()?;
    parse_claude_status_value(&value)
}

pub fn parse_claude_status_cache_json(text: &str) -> Option<SliceProviderAuthSummary> {
    let value: Value = serde_json::from_str(text).ok()?;
    find_claude_status_value(&value)
}

fn find_claude_status_value(value: &Value) -> Option<SliceProviderAuthSummary> {
    if let Some(summary) = parse_claude_status_value(value) {
        return Some(summary);
    }
    match value {
        Value::Array(items) => items.iter().find_map(find_claude_status_value),
        Value::Object(object) => object.values().find_map(find_claude_status_value),
        _ => None,
    }
}

fn parse_claude_status_value(value: &Value) -> Option<SliceProviderAuthSummary> {
    if value.get("loggedIn").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    Some(SliceProviderAuthSummary {
        provider: "claude".to_string(),
        state: SliceProviderAuthState::Authenticated,
        auth_type: value
            .get("authMethod")
            .and_then(Value::as_str)
            .map(str::to_string),
        account_id: None,
        email: value
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_string),
        organization_id: value
            .get("orgId")
            .and_then(Value::as_str)
            .map(str::to_string),
        organization_name: value
            .get("orgName")
            .and_then(Value::as_str)
            .map(str::to_string),
        subscription_type: value
            .get("subscriptionType")
            .and_then(Value::as_str)
            .map(str::to_string),
        alias: None,
        source: "claude_auth_status".to_string(),
    })
}

pub fn parse_claude_settings_json(text: &str) -> Option<SliceProviderAuthSummary> {
    let value: Value = serde_json::from_str(text).ok()?;
    let oauth = value.get("oauthAccount");
    let account_id = value
        .get("userID")
        .and_then(Value::as_str)
        .or_else(|| {
            oauth
                .and_then(|oauth| oauth.get("accountUuid"))
                .and_then(Value::as_str)
        })
        .map(str::to_string);
    let organization_id = oauth
        .and_then(|oauth| oauth.get("organizationUuid"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let subscription_type = oauth
        .and_then(|oauth| oauth.get("billingType"))
        .or_else(|| oauth.and_then(|oauth| oauth.get("organizationType")))
        .and_then(Value::as_str)
        .map(str::to_string);
    if account_id.is_none() && organization_id.is_none() {
        return None;
    }
    Some(SliceProviderAuthSummary {
        provider: "claude".to_string(),
        state: SliceProviderAuthState::Configured,
        auth_type: Some("claude.ai".to_string()),
        account_id,
        email: None,
        organization_id,
        organization_name: None,
        subscription_type,
        alias: None,
        source: "home_claude_settings_json".to_string(),
    })
}

pub fn merge_provider_auth_summaries(
    summaries: Vec<SliceProviderAuthSummary>,
) -> Vec<SliceProviderAuthSummary> {
    let mut merged: Vec<SliceProviderAuthSummary> = Vec::new();
    for summary in summaries {
        if let Some(existing) = merged
            .iter_mut()
            .find(|existing| existing.provider == summary.provider)
        {
            merge_provider_auth_summary(existing, summary);
        } else {
            merged.push(summary);
        }
    }
    merged
}

fn merge_provider_auth_summary(
    existing: &mut SliceProviderAuthSummary,
    candidate: SliceProviderAuthSummary,
) {
    let candidate_is_better =
        provider_auth_summary_quality(&candidate) > provider_auth_summary_quality(existing);
    if candidate_is_better {
        let previous = existing.clone();
        *existing = candidate;
        fill_missing_provider_auth_fields(existing, previous);
    } else {
        fill_missing_provider_auth_fields(existing, candidate);
    }
}

fn fill_missing_provider_auth_fields(
    target: &mut SliceProviderAuthSummary,
    fallback: SliceProviderAuthSummary,
) {
    fill_missing(&mut target.auth_type, fallback.auth_type);
    fill_missing(&mut target.account_id, fallback.account_id);
    fill_missing(&mut target.email, fallback.email);
    fill_missing(&mut target.organization_id, fallback.organization_id);
    fill_missing(&mut target.organization_name, fallback.organization_name);
    fill_missing(&mut target.subscription_type, fallback.subscription_type);
    fill_missing(&mut target.alias, fallback.alias);
    if !target.source.contains(&fallback.source) {
        target.source = format!("{}+{}", target.source, fallback.source);
    }
}

fn fill_missing(target: &mut Option<String>, fallback: Option<String>) {
    if target.is_none() {
        *target = fallback;
    }
}

fn provider_auth_summary_quality(summary: &SliceProviderAuthSummary) -> u8 {
    state_quality(&summary.state) + identity_quality(summary)
}

fn state_quality(state: &SliceProviderAuthState) -> u8 {
    match state {
        SliceProviderAuthState::Unknown => 0,
        SliceProviderAuthState::NotConfigured => 1,
        SliceProviderAuthState::Configured => 2,
        SliceProviderAuthState::Authenticated => 4,
    }
}

fn identity_quality(summary: &SliceProviderAuthSummary) -> u8 {
    [
        summary.alias.as_ref(),
        summary.email.as_ref(),
        summary.account_id.as_ref(),
        summary.organization_id.as_ref(),
        summary.organization_name.as_ref(),
        summary.subscription_type.as_ref(),
    ]
    .into_iter()
    .filter(|value| value.is_some())
    .count() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_parser_extracts_account_without_secret() {
        let summary = parse_codex_auth_json(
            r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":"secret","tokens":{"account_id":"acct-1"}}"#,
        )
        .expect("codex auth should parse");

        assert_eq!(summary.provider, "codex");
        assert_eq!(summary.auth_type.as_deref(), Some("chatgpt"));
        assert_eq!(summary.account_id.as_deref(), Some("acct-1"));
        assert_eq!(
            serde_json::to_string(&summary).unwrap().contains("secret"),
            false
        );
    }

    #[test]
    fn opencode_parser_extracts_multiple_provider_accounts() {
        let summaries = parse_opencode_auth_json(
            r#"{"openai":{"type":"oauth","accountId":"acct-1","accessToken":"secret-token"},"opencode":{"type":"api","apiKey":"secret-key"}}"#,
        );

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].provider, "opencode:openai");
        assert_eq!(summaries[0].account_id.as_deref(), Some("acct-1"));
        assert_eq!(summaries[1].provider, "opencode:opencode");
        assert_eq!(summaries[1].auth_type.as_deref(), Some("api"));
        let serialized = serde_json::to_string(&summaries).unwrap();
        assert!(!serialized.contains("secret-token"));
        assert!(!serialized.contains("secret-key"));
    }

    #[test]
    fn claude_status_parser_extracts_email_when_available() {
        let summary = parse_claude_status_json(
            r#"{"loggedIn":true,"authMethod":"claude.ai","email":"user@example.com","orgId":"org-1","orgName":"Example Org","subscriptionType":"pro"}"#,
        )
        .expect("claude status should parse");

        assert_eq!(summary.provider, "claude");
        assert_eq!(summary.state, SliceProviderAuthState::Authenticated);
        assert_eq!(summary.email.as_deref(), Some("user@example.com"));
        assert_eq!(summary.organization_id.as_deref(), Some("org-1"));
    }

    #[test]
    fn claude_settings_parser_extracts_account_when_cli_status_is_unavailable() {
        let summary = parse_claude_settings_json(
            r#"{"userID":"user-1","oauthAccount":{"accountUuid":"acct-1","organizationUuid":"org-1","billingType":"stripe_subscription","accessToken":"secret-token"}}"#,
        )
        .expect("claude settings should parse");

        assert_eq!(summary.provider, "claude");
        assert_eq!(summary.account_id.as_deref(), Some("user-1"));
        assert_eq!(summary.organization_id.as_deref(), Some("org-1"));
        assert!(!serde_json::to_string(&summary)
            .unwrap()
            .contains("secret-token"));
    }

    #[test]
    fn claude_status_cache_parser_finds_nested_authenticated_identity() {
        let summary = parse_claude_status_cache_json(
            r#"{"cache":{"auth":{"loggedIn":true,"authMethod":"claude.ai","email":"user@example.com","orgId":"org-1","orgName":"Example Org","subscriptionType":"team","refreshToken":"secret"}}}"#,
        )
        .expect("nested claude status cache should parse");

        assert_eq!(summary.provider, "claude");
        assert_eq!(summary.state, SliceProviderAuthState::Authenticated);
        assert_eq!(summary.email.as_deref(), Some("user@example.com"));
        assert_eq!(summary.organization_name.as_deref(), Some("Example Org"));
        assert!(!serde_json::to_string(&summary).unwrap().contains("secret"));
    }

    #[test]
    fn home_provider_auth_merges_claude_settings_with_status_cache() {
        let home = unique_test_home("slice-provider-auth-claude-merge");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            home.join(".claude.json"),
            r#"{"userID":"user-1","oauthAccount":{"organizationUuid":"org-1","billingType":"pro"}}"#,
        )
        .unwrap();
        std::fs::write(
            home.join(".claude").join("stats-cache.json"),
            r#"{"auth":{"loggedIn":true,"authMethod":"claude.ai","email":"user@example.com","orgName":"Example Org"}}"#,
        )
        .unwrap();

        let summaries = inspect_home_provider_auth(&home);
        let claude = summaries
            .iter()
            .find(|summary| summary.provider == "claude")
            .expect("claude summary should be present");

        assert_eq!(claude.state, SliceProviderAuthState::Authenticated);
        assert_eq!(claude.email.as_deref(), Some("user@example.com"));
        assert_eq!(claude.account_id.as_deref(), Some("user-1"));
        assert_eq!(claude.organization_id.as_deref(), Some("org-1"));
        assert_eq!(claude.organization_name.as_deref(), Some("Example Org"));

        let _ = std::fs::remove_dir_all(home);
    }

    fn unique_test_home(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn inspect_home_provider_auth_caches_until_explicitly_cleared() {
        let home = unique_test_home("slice-provider-auth-cache");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(
            home.join(".codex").join("auth.json"),
            r#"{"auth_mode":"api_key","OPENAI_API_KEY":"sk-test"}"#,
        )
        .unwrap();

        let first = inspect_home_provider_auth(&home);
        assert!(first.iter().any(|summary| summary.provider == "codex"));

        // Removing the auth file does not change the cached result within TTL.
        std::fs::remove_file(home.join(".codex").join("auth.json")).unwrap();
        let cached = inspect_home_provider_auth(&home);
        assert_eq!(cached, first, "scan should be served from cache");

        // The uncached reader always reflects the current files.
        assert!(read_home_provider_auth(&home).is_empty());

        // Clearing the cache forces a fresh read that sees the removal.
        clear_home_provider_auth_cache();
        assert!(inspect_home_provider_auth(&home).is_empty());

        let _ = std::fs::remove_dir_all(home);
    }
}
