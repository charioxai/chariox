use serde::Deserialize;

pub(crate) const MIN_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS: u64 = 100;
pub(crate) const MAX_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS: u64 = 5_000;
pub(crate) const DEFAULT_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS: u64 = 5_000;
const MAX_BROWSER_COMPATIBILITY_SELECTOR_BYTES: usize = 8_192;
const MAX_BROWSER_COMPATIBILITY_URL_BYTES: usize = 8_192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserCompatibilityWait {
    Selector(String),
    Idle,
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct BrowserControllerNavigationResult {
    pub(crate) browser_generation: u64,
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) url: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
