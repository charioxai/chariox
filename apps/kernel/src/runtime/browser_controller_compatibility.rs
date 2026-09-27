use serde::{Deserialize, Serialize};

pub(crate) const MIN_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS: u64 = 100;
pub(crate) const MAX_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS: u64 = 5_000;
pub(crate) const DEFAULT_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS: u64 = 5_000;
const MAX_BROWSER_COMPATIBILITY_SELECTOR_BYTES: usize = 8_192;
const MAX_BROWSER_COMPATIBILITY_URL_BYTES: usize = 8_192;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "selector", rename_all = "snake_case")]
pub(crate) enum BrowserCompatibilityWait {
    Selector(String),
    Idle,
}

impl std::fmt::Debug for BrowserCompatibilityWait {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Selector(_) => formatter
                .debug_struct("BrowserCompatibilityWait")
                .field("kind", &"selector")
                .field("selector", &"[redacted]")
                .finish(),
            Self::Idle => formatter
                .debug_struct("BrowserCompatibilityWait")
                .field("kind", &"idle")
                .finish(),
        }
    }
}

impl BrowserCompatibilityWait {
    pub(crate) fn validate(&self, timeout_ms: u64) -> Result<(), String> {
        if !(MIN_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS..=MAX_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS)
            .contains(&timeout_ms)
        {
            return Err(format!(
                "browser compatibility wait timeout must be between {MIN_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS} and {MAX_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS} milliseconds"
            ));
        }
        match self {
            Self::Selector(selector)
                if selector.is_empty()
                    || selector.len() > MAX_BROWSER_COMPATIBILITY_SELECTOR_BYTES =>
            {
                Err(format!(
                    "browser compatibility selector must be between 1 and {MAX_BROWSER_COMPATIBILITY_SELECTOR_BYTES} UTF-8 bytes"
                ))
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Selector(_) => "selector",
            Self::Idle => "idle",
        }
    }

    pub(crate) fn selector(&self) -> Option<&str> {
        match self {
            Self::Selector(selector) => Some(selector),
            Self::Idle => None,
        }
    }
}

pub(crate) fn normalize_browser_navigation_url(url: &str) -> Result<String, String> {
    if url.is_empty() || url.len() > MAX_BROWSER_COMPATIBILITY_URL_BYTES {
        return Err(format!(
            "browser navigation URL must be between 1 and {MAX_BROWSER_COMPATIBILITY_URL_BYTES} UTF-8 bytes"
        ));
    }
    let parsed = url::Url::parse(url)
        .map_err(|error| format!("browser navigation URL must be absolute: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("browser navigation URL must use HTTP or HTTPS".to_string());
    }
    Ok(parsed.to_string())
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct BrowserNavigationUrl(String);

impl BrowserNavigationUrl {
    pub(crate) fn new(url: &str) -> Result<Self, String> {
        normalize_browser_navigation_url(url).map(Self)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for BrowserNavigationUrl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted browser navigation URL]")
    }
}

impl TryFrom<String> for BrowserNavigationUrl {
    type Error = String;

    fn try_from(url: String) -> Result<Self, Self::Error> {
        Self::new(&url)
    }
}

impl From<BrowserNavigationUrl> for String {
    fn from(url: BrowserNavigationUrl) -> Self {
        url.0
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerNavigationResult {
    pub(crate) browser_generation: u64,
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) url: String,
}

impl std::fmt::Debug for BrowserControllerNavigationResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserControllerNavigationResult")
            .field("browser_generation", &self.browser_generation)
            .field("target_id", &self.target_id)
            .field("document_id", &self.document_id)
            .field("url", &"[redacted]")
            .finish()
    }
}

impl BrowserControllerNavigationResult {
    pub(crate) fn validate(&self, target_id: &str, expected_url: &str) -> Result<(), String> {
        if self.browser_generation == 0 || self.document_id.is_empty() {
            return Err(
                "browser controller navigation returned an invalid generation or document identity"
                    .to_string(),
            );
        }
        if self.target_id != target_id || self.url != expected_url {
            return Err("browser controller navigation changed its target or URL".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerCompatibilityWaitResult {
    pub(crate) browser_generation: u64,
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) kind: String,
    pub(crate) ok: bool,
    pub(crate) elapsed_ms: u64,
}

impl BrowserControllerCompatibilityWaitResult {
    pub(crate) fn validate(
        &self,
        target_id: &str,
        document_id: &str,
        wait: &BrowserCompatibilityWait,
    ) -> Result<(), String> {
        if self.browser_generation == 0 || !self.ok {
            return Err(
                "browser controller compatibility wait returned an invalid terminal state"
                    .to_string(),
            );
        }
        if self.target_id != target_id || self.document_id != document_id {
            return Err(
                "browser controller compatibility wait changed document identity".to_string(),
            );
        }
        if self.kind != wait.kind() {
            return Err("browser controller compatibility wait changed wait kind".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_inputs_and_results_are_bounded_and_identity_checked() {
        assert!(normalize_browser_navigation_url("file:///tmp/private").is_err());
        assert_eq!(
            normalize_browser_navigation_url("https://example.test/settings")
                .expect("HTTPS navigation should normalize"),
            "https://example.test/settings"
        );
        assert!(BrowserCompatibilityWait::Selector(String::new())
            .validate(DEFAULT_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS)
            .is_err());
        assert!(BrowserCompatibilityWait::Idle
            .validate(MAX_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS + 1)
            .is_err());
        let navigation = BrowserNavigationUrl::new(
            "https://example.test/path?sensitive-navigation-unit-fixture",
        )
        .expect("bounded navigation URL");
        assert_eq!(
            serde_json::to_value(&navigation).expect("navigation URL serialization"),
            serde_json::json!("https://example.test/path?sensitive-navigation-unit-fixture")
        );
        assert!(!format!("{navigation:?}").contains("sensitive-navigation-unit-fixture"));
        let selector = BrowserCompatibilityWait::Selector(
            "[data-secret='sensitive-selector-unit-fixture']".into(),
        );
        assert_eq!(
            serde_json::to_value(&selector).expect("selector wait serialization"),
            serde_json::json!({
                "kind":"selector",
                "selector":"[data-secret='sensitive-selector-unit-fixture']"
            })
        );
        assert!(!format!("{selector:?}").contains("sensitive-selector-unit-fixture"));

        let result = BrowserControllerCompatibilityWaitResult {
            browser_generation: 1,
            target_id: "target-a".to_string(),
            document_id: "loader-a".to_string(),
            kind: "idle".to_string(),
            ok: true,
            elapsed_ms: 7,
        };
        assert!(result
            .validate("target-a", "loader-a", &BrowserCompatibilityWait::Idle)
            .is_ok());
        assert!(result
            .validate("target-a", "loader-b", &BrowserCompatibilityWait::Idle)
            .is_err());
    }
}
