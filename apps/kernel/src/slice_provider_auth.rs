use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    if let Ok(text) = std::fs::read_to_string(home_dir.join(".pi").join("agent").join("auth.json"))
    {
        summaries.extend(parse_pi_auth_json(&text));
    }
    if let Ok(text) = std::fs::read_to_string(home_dir.join(".pi").join("auth.json")) {
        summaries.extend(parse_pi_auth_json(&text));
    }
    merge_provider_auth_summaries(summaries)
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

pub fn parse_pi_auth_json(text: &str) -> Vec<SliceProviderAuthSummary> {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    object
        .iter()
        .filter_map(|(provider, config)| parse_pi_auth_provider(provider, config))
        .collect()
}

fn parse_pi_auth_provider(provider: &str, config: &Value) -> Option<SliceProviderAuthSummary> {
    if config.is_null() {
        return None;
    }
    let provider = normalize_pi_auth_provider(provider);
    let auth_type = string_field(
        config,
        &["type", "auth_type", "authType", "mode", "auth_mode"],
    );
    let account_id = string_field(
        config,
        &[
            "accountId",
            "account_id",
            "userId",
            "user_id",
            "id",
            "organizationId",
            "organization_id",
        ],
    );
    let email = string_field(config, &["email", "login", "username"]);
    let configured = match config.as_object() {
        Some(object) => !object.is_empty(),
        None => config
            .as_str()
            .is_some_and(|value| !value.trim().is_empty()),
    };
    if !configured && auth_type.is_none() && account_id.is_none() && email.is_none() {
        return None;
    }
    Some(SliceProviderAuthSummary {
        provider: format!("pi:{provider}"),
        state: SliceProviderAuthState::Configured,
        auth_type,
        account_id,
        email,
        organization_id: None,
        organization_name: None,
        subscription_type: None,
        alias: None,
        source: "home_pi_auth_json".to_string(),
    })
}

fn normalize_pi_auth_provider(provider: &str) -> String {
    match provider {
        "claude" => "anthropic".to_string(),
        "codex" | "openai-codex" => "openai-codex".to_string(),
        "copilot" | "autopilot" => "github-copilot".to_string(),
        value => value.to_string(),
    }
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::to_string)
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
    fn pi_parser_extracts_backing_provider_accounts_without_secrets() {
        let summaries = parse_pi_auth_json(
            r#"{"openai-codex":{"type":"oauth","accountId":"acct-1","accessToken":"secret-token"},"claude":{"apiKey":"secret-key","email":"user@example.com"}}"#,
        );

        assert_eq!(summaries.len(), 2);
        let openai = summaries
            .iter()
            .find(|summary| summary.provider == "pi:openai-codex")
            .expect("openai-codex Pi backing provider summary");
        let anthropic = summaries
            .iter()
            .find(|summary| summary.provider == "pi:anthropic")
            .expect("anthropic Pi backing provider summary");
        assert_eq!(openai.account_id.as_deref(), Some("acct-1"));
        assert_eq!(anthropic.email.as_deref(), Some("user@example.com"));
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
}
