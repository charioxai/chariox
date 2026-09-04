use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowserTabAction {
    Activate,
    Close,
}

impl BrowserTabAction {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Activate => "activate",
            Self::Close => "close",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerTabResult {
    pub(crate) browser_generation: u64,
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) action: BrowserTabAction,
}

impl BrowserControllerTabResult {
    pub(crate) fn validate(
        &self,
        target_id: &str,
        document_id: &str,
        action: BrowserTabAction,
    ) -> Result<(), String> {
        if self.browser_generation == 0 {
            return Err("browser tab operation returned a zero generation".to_string());
        }
        if self.target_id != target_id || self.document_id != document_id {
            return Err("browser tab operation changed target or document identity".to_string());
        }
        if self.action != action {
            return Err("browser tab operation changed action".to_string());
        }
        Ok(())
    }
}
