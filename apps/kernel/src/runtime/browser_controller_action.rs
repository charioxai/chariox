use serde::Deserialize;

const MAX_FILL_TEXT_BYTES: usize = 65_536;
const MAX_DIALOG_PROMPT_TEXT_BYTES: usize = 2_048;
pub(crate) const MIN_BROWSER_ACTION_TIMEOUT_MS: u64 = 100;
pub(crate) const MAX_BROWSER_ACTION_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserLocatorAction {
    Click,
    Fill {
        text: String,
        append: bool,
        submit: bool,
    },
    Submit,
}

impl BrowserLocatorAction {
    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            Self::Click | Self::Submit => Ok(()),
            Self::Fill { text, .. } if text.len() <= MAX_FILL_TEXT_BYTES => Ok(()),
            Self::Fill { .. } => Err(format!(
                "browser fill text exceeds {MAX_FILL_TEXT_BYTES} UTF-8 bytes"
            )),
        }
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Fill { .. } => "fill",
            Self::Submit => "submit",
        }
    }

    pub(crate) fn controller_value(&self) -> serde_json::Value {
        match self {
            Self::Click => serde_json::json!({ "kind": "click" }),
            Self::Fill {
                text,
                append,
                submit,
            } => serde_json::json!({
                "kind": "fill",
                "text": text,
                "append": append,
                "submit": submit,
            }),
            Self::Submit => serde_json::json!({ "kind": "submit" }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct BrowserControllerActionResult {
    pub(crate) browser_generation: u64,
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) action_kind: String,
    #[serde(default)]
    pub(crate) dialog_opened: bool,
    pub(crate) attempts: u64,
    pub(crate) elapsed_ms: u64,
}

impl BrowserControllerActionResult {
    pub(crate) fn validate(
        &self,
        expected_target_id: &str,
        expected_document_id: &str,
        expected_action_kind: &str,
    ) -> Result<(), String> {
        if self.browser_generation == 0 || self.attempts == 0 {
            return Err(
                "browser controller action returned a zero generation or attempt count".to_string(),
            );
        }
        if self.target_id != expected_target_id || self.document_id != expected_document_id {
            return Err(
                "browser controller action changed target or document identity".to_string(),
            );
        }
        if self.action_kind != expected_action_kind {
            return Err("browser controller action changed action kind".to_string());
        }
        Ok(())
    }

    pub(crate) fn into_room_result(
        self,
        session_id: String,
        environment_id: String,
        runtime_generation: u64,
        tab_id: String,
        document_revision: u64,
        element_ref: String,
    ) -> RoomBrowserActionResult {
        RoomBrowserActionResult {
            session_id,
            environment_id,
            runtime_generation,
            tab_id,
            document_revision,
            element_ref,
            action_kind: self.action_kind,
            dialog_opened: self.dialog_opened,
            attempts: self.attempts,
            elapsed_ms: self.elapsed_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoomBrowserActionResult {
    pub(crate) session_id: String,
    pub(crate) environment_id: String,
    pub(crate) runtime_generation: u64,
    pub(crate) tab_id: String,
    pub(crate) document_revision: u64,
    pub(crate) element_ref: String,
    pub(crate) action_kind: String,
    pub(crate) dialog_opened: bool,
    pub(crate) attempts: u64,
    pub(crate) elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserDialogAction {
    Accept { prompt_text: Option<String> },
    Dismiss,
}

impl BrowserDialogAction {
    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            Self::Accept {
                prompt_text: Some(text),
            } if text.len() > MAX_DIALOG_PROMPT_TEXT_BYTES => Err(format!(
                "browser dialog prompt text exceeds {MAX_DIALOG_PROMPT_TEXT_BYTES} UTF-8 bytes"
            )),
            _ => Ok(()),
        }
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Accept { .. } => "accept",
            Self::Dismiss => "dismiss",
        }
    }

    pub(crate) fn prompt_text(&self) -> Option<&str> {
        match self {
            Self::Accept { prompt_text } => prompt_text.as_deref(),
            Self::Dismiss => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct BrowserControllerDialogResult {
    pub(crate) browser_generation: u64,
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) action: String,
}

impl BrowserControllerDialogResult {
    pub(crate) fn validate(
        &self,
        target_id: &str,
        document_id: &str,
        action: &BrowserDialogAction,
    ) -> Result<(), String> {
        if self.browser_generation == 0 {
            return Err("browser controller dialog returned a zero generation".to_string());
        }
        if self.target_id != target_id || self.document_id != document_id {
            return Err(
                "browser controller dialog changed target or document identity".to_string(),
            );
        }
        if self.action != action.kind() {
            return Err("browser controller dialog changed action kind".to_string());
        }
        Ok(())
    }

    pub(crate) fn into_room_result(
        self,
        session_id: String,
        environment_id: String,
        runtime_generation: u64,
        tab_id: String,
        document_revision: u64,
    ) -> RoomBrowserDialogResult {
        RoomBrowserDialogResult {
            session_id,
            environment_id,
            runtime_generation,
            tab_id,
            document_revision,
            action: self.action,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoomBrowserDialogResult {
    pub(crate) session_id: String,
    pub(crate) environment_id: String,
    pub(crate) runtime_generation: u64,
    pub(crate) tab_id: String,
    pub(crate) document_revision: u64,
    pub(crate) action: String,
}

pub(crate) fn validate_browser_action_timeout(timeout_ms: u64) -> Result<(), String> {
    if (MIN_BROWSER_ACTION_TIMEOUT_MS..=MAX_BROWSER_ACTION_TIMEOUT_MS).contains(&timeout_ms) {
        Ok(())
    } else {
        Err(format!(
            "browser action timeout must be between {MIN_BROWSER_ACTION_TIMEOUT_MS} and {MAX_BROWSER_ACTION_TIMEOUT_MS} milliseconds"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_actions_bound_fill_text_timeout_and_controller_identity() {
        assert!(BrowserLocatorAction::Fill {
            text: "😀".repeat(20_000),
            append: false,
            submit: false,
        }
        .validate()
        .is_err());
        assert!(validate_browser_action_timeout(99).is_err());
        assert!(validate_browser_action_timeout(5_001).is_err());
        assert!(BrowserDialogAction::Accept {
            prompt_text: Some("😀".repeat(600)),
        }
        .validate()
        .is_err());

        let mut result = BrowserControllerActionResult {
            browser_generation: 1,
            target_id: "target-a".to_string(),
            document_id: "loader-a".to_string(),
            action_kind: "click".to_string(),
            dialog_opened: true,
            attempts: 2,
            elapsed_ms: 50,
        };
        assert!(result.validate("target-a", "loader-a", "click").is_ok());
        result.document_id = "loader-b".to_string();
        assert!(result.validate("target-a", "loader-a", "click").is_err());
    }
}
