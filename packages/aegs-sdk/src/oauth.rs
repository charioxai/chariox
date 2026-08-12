use std::collections::HashMap;

use arroba_event_protocol::AegsAuthorizationFlow;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::{
    digest, environment, http_json, now_ms, parse_base_url, parse_public_base_url, parse_url,
    random_opaque, read_secret, AegsStore, AuthorizationCallback, CredentialCipher,
    AUTHORIZATION_TTL_MS, PROVIDER_HTTP_TIMEOUT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthTokenProtocol {
    StandardForm,
    AtlassianJson,
    SlackForm,
}

#[derive(Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub authorization_url: Url,
    pub token_url: Url,
    pub api_base_url: Url,
    pub scopes: String,
    pub token_protocol: OAuthTokenProtocol,
    pub authorization_parameters: Vec<(String, String)>,
}

#[derive(Clone)]
pub struct OAuthDefaults {
    pub authorization_url: String,
    pub token_url: String,
    pub api_base_url: String,
    pub scopes: String,
    pub token_protocol: OAuthTokenProtocol,
    pub authorization_parameters: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCredential {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Clone)]
pub struct OAuthAuthorization {
    generator_id: &'static str,
    provider_slug: &'static str,
    store: AegsStore,
    public_base_url: Url,
    cipher: CredentialCipher,
    config: OAuthConfig,
}

impl OAuthAuthorization {
    pub fn from_environment(
        generator_id: &'static str,
        provider_slug: &'static str,
        store: AegsStore,
        defaults: OAuthDefaults,
    ) -> Result<Option<Self>, String> {
        let public_base_url = environment("ARROBA_AEGS_PUBLIC_BASE_URL");
        let credential_key = read_secret(
            "ARROBA_AEGS_CREDENTIAL_KEY",
            "ARROBA_AEGS_CREDENTIAL_KEY_FILE",
        )?;
        let client_id = environment("ARROBA_AEGS_OAUTH_CLIENT_ID");
        let client_secret = read_secret(
            "ARROBA_AEGS_OAUTH_CLIENT_SECRET",
            "ARROBA_AEGS_OAUTH_CLIENT_SECRET_FILE",
        )?;
        match (public_base_url, credential_key, client_id, client_secret) {
            (None, None, None, None) => Ok(None),
            (
                Some(public_base_url),
                Some(credential_key),
                Some(client_id),
                Some(client_secret),
            ) => Ok(Some(Self::new(
                generator_id,
                provider_slug,
                store,
                parse_public_base_url(&public_base_url)?,
                CredentialCipher::parse(&credential_key)?,
                OAuthConfig {
                    client_id,
                    client_secret,
                    authorization_url: parse_url(
                        environment("ARROBA_AEGS_OAUTH_AUTHORIZATION_URL")
                            .as_deref()
                            .unwrap_or(&defaults.authorization_url),
                    )?,
                    token_url: parse_url(
                        environment("ARROBA_AEGS_OAUTH_TOKEN_URL")
                            .as_deref()
                            .unwrap_or(&defaults.token_url),
                    )?,
                    api_base_url: parse_base_url(
                        environment("ARROBA_AEGS_API_URL")
                            .as_deref()
                            .unwrap_or(&defaults.api_base_url),
                    )?,
                    scopes: environment("ARROBA_AEGS_OAUTH_SCOPES")
                        .unwrap_or(defaults.scopes),
                    token_protocol: defaults.token_protocol,
                    authorization_parameters: defaults.authorization_parameters,
                },
            ))),
            _ => Err(
                "OAuth authorization is partially configured; public URL, credential key, client ID, and client secret are all required"
                    .to_string(),
            ),
        }
    }

    pub fn new(
        generator_id: &'static str,
        provider_slug: &'static str,
        store: AegsStore,
        public_base_url: Url,
        cipher: CredentialCipher,
        config: OAuthConfig,
    ) -> Self {
        Self {
            generator_id,
            provider_slug,
            store,
            public_base_url,
            cipher,
            config,
        }
    }

    pub fn config(&self) -> &OAuthConfig {
        &self.config
    }

    pub fn public_base_url(&self) -> &Url {
        &self.public_base_url
    }

    pub fn store(&self) -> &AegsStore {
        &self.store
    }

    pub fn start(
        &self,
        owner_id: &str,
        return_url: Option<&str>,
    ) -> Result<AegsAuthorizationFlow, String> {
        self.start_for_connection(owner_id, random_opaque("connection"), return_url, false)
    }

    pub fn reconnect(
        &self,
        owner_id: &str,
        connection_id: &str,
        return_url: Option<&str>,
    ) -> Result<AegsAuthorizationFlow, String> {
        self.start_for_connection(owner_id, connection_id.to_string(), return_url, true)
    }

    fn start_for_connection(
        &self,
        owner_id: &str,
        connection_id: String,
        return_url: Option<&str>,
        reconnect: bool,
    ) -> Result<AegsAuthorizationFlow, String> {
        let now = now_ms();
        let expires_at_ms = now.saturating_add(AUTHORIZATION_TTL_MS);
        let state = random_opaque("state");
        if reconnect {
            self.store.create_reauthorization(
                &digest(&state),
                &connection_id,
                owner_id,
                self.provider_slug,
                return_url,
                expires_at_ms,
                now,
            )?;
        } else {
            self.store.create_authorization(
                &digest(&state),
                &connection_id,
                owner_id,
                self.provider_slug,
                return_url,
                expires_at_ms,
                now,
            )?;
        }
        let callback_url = self.callback_url()?;
        let mut authorization_url = self.config.authorization_url.clone();
        {
            let mut query = authorization_url.query_pairs_mut();
            query
                .append_pair("client_id", &self.config.client_id)
                .append_pair("redirect_uri", callback_url.as_str())
                .append_pair("response_type", "code")
                .append_pair("state", &state)
                .append_pair("scope", &self.config.scopes);
            for (key, value) in &self.config.authorization_parameters {
                query.append_pair(key, value);
            }
        }
        Ok(AegsAuthorizationFlow {
            generator_id: self.generator_id.to_string(),
            status: "user_action_required".to_string(),
            connection_id: Some(connection_id),
            authorization_url: Some(authorization_url.to_string()),
            user_code: None,
            expires_at_ms: Some(expires_at_ms),
        })
    }

    pub fn complete<F>(
        &self,
        query: &HashMap<String, String>,
        metadata: F,
    ) -> Result<AuthorizationCallback, String>
    where
        F: FnOnce(&Value) -> Value,
    {
        if let Some(error) = query.get("error") {
            return Err(format!(
                "provider authorization failed: {}",
                query
                    .get("error_description")
                    .map(String::as_str)
                    .unwrap_or(error)
            ));
        }
        let state = query
            .get("state")
            .ok_or_else(|| "authorization callback is missing state".to_string())?;
        let state_digest = digest(state);
        let pending = self
            .store
            .authorization(&state_digest, now_ms())?
            .ok_or_else(|| "authorization state is invalid or expired".to_string())?;
        if pending.provider != self.provider_slug {
            return Err("authorization state belongs to another provider".to_string());
        }
        let code = query
            .get("code")
            .ok_or_else(|| "authorization callback is missing code".to_string())?;
        let token = self.exchange_code(code)?;
        let expires_at_ms = token
            .get("expires_in")
            .and_then(Value::as_u64)
            .map(|seconds| now_ms().saturating_add(seconds.saturating_mul(1_000)));
        let credential = self.credential_from_token(&token)?;
        let encrypted = self.cipher.encrypt(&credential)?;
        let connection = self.store.complete_authorization(
            &state_digest,
            &encrypted,
            &metadata(&token),
            expires_at_ms,
            now_ms(),
        )?;
        Ok(AuthorizationCallback {
            connection_id: connection.connection_id,
            return_url: pending.return_url,
        })
    }

    pub fn ready_credential(&self, connection_id: &str) -> Result<OAuthCredential, String> {
        let connection = self
            .store
            .connection(connection_id)?
            .ok_or_else(|| "the authorized connection was not found".to_string())?;
        if connection.provider != self.provider_slug {
            return Err("the connection belongs to another provider".to_string());
        }
        if connection.status != "ready" {
            return Err("authorization_pending".to_string());
        }
        let encrypted = connection
            .encrypted_credential
            .as_deref()
            .ok_or_else(|| "the authorized connection has no credential".to_string())?;
        let mut credential: OAuthCredential = self.cipher.decrypt(encrypted)?;
        if connection
            .expires_at_ms
            .is_some_and(|expires| expires <= now_ms().saturating_add(60_000))
        {
            let (refreshed, expires_at_ms) = self.refresh(&credential)?;
            let encrypted = self.cipher.encrypt(&refreshed)?;
            self.store.update_connection_credential(
                &connection.connection_id,
                &encrypted,
                expires_at_ms,
                now_ms(),
            )?;
            credential = refreshed;
        }
        Ok(credential)
    }

    fn callback_url(&self) -> Result<Url, String> {
        self.public_base_url
            .join("oauth/callback")
            .map_err(|error| error.to_string())
    }

    fn exchange_code(&self, code: &str) -> Result<Value, String> {
        let callback_url = self.callback_url()?;
        let value = match self.config.token_protocol {
            OAuthTokenProtocol::AtlassianJson => http_json(
                ureq::post(self.config.token_url.as_str())
                    .timeout(PROVIDER_HTTP_TIMEOUT)
                    .set("content-type", "application/json")
                    .send_json(serde_json::json!({
                        "grant_type": "authorization_code",
                        "client_id": self.config.client_id,
                        "client_secret": self.config.client_secret,
                        "code": code,
                        "redirect_uri": callback_url.as_str(),
                    })),
            )?,
            OAuthTokenProtocol::StandardForm | OAuthTokenProtocol::SlackForm => http_json(
                ureq::post(self.config.token_url.as_str())
                    .timeout(PROVIDER_HTTP_TIMEOUT)
                    .set("accept", "application/json")
                    .send_form(&[
                        ("grant_type", "authorization_code"),
                        ("client_id", self.config.client_id.as_str()),
                        ("client_secret", self.config.client_secret.as_str()),
                        ("code", code),
                        ("redirect_uri", callback_url.as_str()),
                    ]),
            )?,
        };
        self.validate_token_response(&value, "exchange")?;
        Ok(value)
    }

    fn refresh(&self, current: &OAuthCredential) -> Result<(OAuthCredential, Option<u64>), String> {
        let refresh_token = current
            .refresh_token
            .as_deref()
            .ok_or_else(|| "provider connection expired and has no refresh token".to_string())?;
        let response = match self.config.token_protocol {
            OAuthTokenProtocol::AtlassianJson => http_json(
                ureq::post(self.config.token_url.as_str())
                    .timeout(PROVIDER_HTTP_TIMEOUT)
                    .set("content-type", "application/json")
                    .send_json(serde_json::json!({
                        "grant_type": "refresh_token",
                        "client_id": self.config.client_id,
                        "client_secret": self.config.client_secret,
                        "refresh_token": refresh_token,
                    })),
            )?,
            OAuthTokenProtocol::StandardForm | OAuthTokenProtocol::SlackForm => http_json(
                ureq::post(self.config.token_url.as_str())
                    .timeout(PROVIDER_HTTP_TIMEOUT)
                    .set("accept", "application/json")
                    .send_form(&[
                        ("grant_type", "refresh_token"),
                        ("client_id", self.config.client_id.as_str()),
                        ("client_secret", self.config.client_secret.as_str()),
                        ("refresh_token", refresh_token),
                    ]),
            )?,
        };
        self.validate_token_response(&response, "refresh")?;
        let mut refreshed = self.credential_from_token(&response)?;
        if refreshed.refresh_token.is_none() {
            refreshed.refresh_token = current.refresh_token.clone();
        }
        if refreshed.scope.is_none() {
            refreshed.scope = current.scope.clone();
        }
        let expires_at_ms = response
            .get("expires_in")
            .and_then(Value::as_u64)
            .map(|seconds| now_ms().saturating_add(seconds.saturating_mul(1_000)));
        Ok((refreshed, expires_at_ms))
    }

    fn validate_token_response(&self, value: &Value, operation: &str) -> Result<(), String> {
        if self.config.token_protocol == OAuthTokenProtocol::SlackForm
            && value.get("ok").and_then(Value::as_bool) != Some(true)
        {
            return Err(format!(
                "Slack token {operation} failed: {}",
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
            ));
        }
        Ok(())
    }

    fn credential_from_token(&self, token: &Value) -> Result<OAuthCredential, String> {
        let access_token = if self.config.token_protocol == OAuthTokenProtocol::SlackForm {
            token
                .get("access_token")
                .or_else(|| token.pointer("/authed_user/access_token"))
        } else {
            token.get("access_token")
        }
        .and_then(Value::as_str)
        .ok_or_else(|| "provider token response is missing access_token".to_string())?;
        Ok(OAuthCredential {
            access_token: access_token.to_string(),
            refresh_token: token
                .get("refresh_token")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            token_type: token
                .get("token_type")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            scope: token
                .get("scope")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_configuration_retains_provider_specific_parameters() {
        let config = OAuthConfig {
            client_id: "client".to_string(),
            client_secret: "secret".to_string(),
            authorization_url: Url::parse("https://provider.test/authorize").unwrap(),
            token_url: Url::parse("https://provider.test/token").unwrap(),
            api_base_url: Url::parse("https://provider.test/api/").unwrap(),
            scopes: "read".to_string(),
            token_protocol: OAuthTokenProtocol::StandardForm,
            authorization_parameters: vec![("audience".to_string(), "api".to_string())],
        };
        assert_eq!(config.authorization_parameters[0].0, "audience");
    }
}
