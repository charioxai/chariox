use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowserHistoryAction {
    Back,
    Forward,
    Reload,
}

impl BrowserHistoryAction {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Back => "back",
            Self::Forward => "forward",
            Self::Reload => "reload",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerHistoryResult {
    pub(crate) browser_generation: u64,
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) action: BrowserHistoryAction,
    pub(crate) url: String,
}

impl BrowserControllerHistoryResult {
    pub(crate) fn validate(
        &self,
        target_id: &str,
        action: BrowserHistoryAction,
    ) -> Result<(), String> {
        if self.browser_generation == 0 {
            return Err("browser history operation returned a zero generation".to_string());
        }
        if self.target_id != target_id || self.document_id.is_empty() {
            return Err(
                "browser history operation returned invalid target or document identity"
                    .to_string(),
            );
        }
        if self.action != action {
            return Err("browser history operation changed action".to_string());
        }
        if self.url.is_empty() {
            return Err("browser history operation returned an empty URL".to_string());
        }
        Ok(())
    }
}
