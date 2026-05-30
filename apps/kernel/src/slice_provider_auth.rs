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
    summaries
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
            r#"{"openai":{"type":"oauth","accountId":"acct-1"},"opencode":{"type":"api"}}"#,
        );

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].provider, "opencode:openai");
        assert_eq!(summaries[0].account_id.as_deref(), Some("acct-1"));
        assert_eq!(summaries[1].provider, "opencode:opencode");
        assert_eq!(summaries[1].auth_type.as_deref(), Some("api"));
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
            r#"{"userID":"user-1","oauthAccount":{"accountUuid":"acct-1","organizationUuid":"org-1","billingType":"stripe_subscription"}}"#,
        )
        .expect("claude settings should parse");

        assert_eq!(summary.provider, "claude");
        assert_eq!(summary.account_id.as_deref(), Some("user-1"));
        assert_eq!(summary.organization_id.as_deref(), Some("org-1"));
    }
}
