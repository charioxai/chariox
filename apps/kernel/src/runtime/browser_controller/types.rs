use std::net::SocketAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserTarget {
    pub(crate) endpoint: SocketAddr,
    pub(crate) target_id: String,
    pub(crate) environment_id: String,
    pub(crate) tab_id: String,
    pub(crate) runtime_generation: u64,
}
impl BrowserTarget {
    pub(super) fn validate(&self) -> Result<(), BrowserError> {
        if !self.endpoint.ip().is_loopback()
            || self.endpoint.port() == 0
            || !(1..=i64::MAX as u64).contains(&self.runtime_generation)
            || [&self.target_id, &self.environment_id, &self.tab_id]
                .iter()
                .any(|id| !identifier(id))
        {
            return Err(BrowserError::Invalid);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserReference {
    pub(crate) environment_id: String,
    pub(crate) tab_id: String,
    pub(crate) runtime_generation: u64,
    pub(crate) document_revision: u64,
    pub(crate) controller_epoch: u64,
}
impl BrowserReference {
    pub(super) fn validate(&self, target: &BrowserTarget, epoch: u64) -> Result<(), BrowserError> {
        if self.environment_id != target.environment_id
            || self.tab_id != target.tab_id
            || self.runtime_generation != target.runtime_generation
            || self.document_revision == 0
            || self.controller_epoch != epoch
        {
            return Err(BrowserError::StaleReference);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BrowserInput {
    Pointer { x: u32, y: u32, pressed: bool },
    Text(String),
    Key { key: String, pressed: bool },
}
impl BrowserInput {
    pub(super) fn validate(&self) -> Result<(), BrowserError> {
        match self {
            Self::Pointer { x, y, .. } if *x <= 4096 && *y <= 4096 => Ok(()),
            Self::Text(text) if text.len() <= 4096 && !text.contains('\0') => Ok(()),
            Self::Key { key, .. }
                if matches!(
                    key.as_str(),
                    "Enter"
                        | "Tab"
                        | "Escape"
                        | "Backspace"
                        | "Delete"
                        | "ArrowUp"
                        | "ArrowDown"
                        | "ArrowLeft"
                        | "ArrowRight"
                        | "Home"
                        | "End"
                        | "PageUp"
                        | "PageDown"
                ) =>
            {
                Ok(())
            }
            _ => Err(BrowserError::Invalid),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BrowserSnapshot {
    pub(crate) reference: BrowserReference,
    pub(crate) accessibility: serde_json::Value,
}
#[derive(Debug, Clone)]
pub(crate) struct ScreencastFrame {
    // Chromium supplies no document token with these pixels. This reference is
    // the controller's document observation on receipt, not atomic provenance
    // of the captured image. Input always checks the current document again.
    pub(crate) reference: BrowserReference,
    pub(crate) jpeg: Vec<u8>,
    pub(crate) css_width: u32,
    pub(crate) css_height: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserError {
    Invalid,
    Busy,
    Cancelled,
    Deadline,
    Unavailable,
    StaleReference,
    Protocol,
    OutcomeUncertain,
}
pub(super) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}
