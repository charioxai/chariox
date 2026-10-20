//! MD-N2 / MP-08 / MP-11: same physical observer for host, Room and App pages.
use crate::local::{NoteBox, NoteTextQuote};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserNoteObservation {
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) url: String,
    pub(crate) selection: Option<BrowserNoteSelection>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserNoteSelection {
    pub(crate) quote: NoteTextQuote,
    pub(crate) box_css: NoteBox,
    pub(crate) hint: String,
}
impl BrowserNoteObservation {
    pub(crate) fn validate(
        &self,
        target: Option<&str>,
        document: Option<&str>,
    ) -> Result<(), String> {
        if target.is_some_and(|id| self.target_id != id)
            || document.is_some_and(|id| self.document_id != id)
            || self.target_id.is_empty()
            || self.document_id.is_empty()
            || self.target_id.len() > 512
            || self.document_id.len() > 512
            || self.url.len() > 8192
        {
            return Err("MD-N2: mismatched selection document".into());
        }
        if let Some(s) = &self.selection {
            super::validate_quote(&s.quote)?;
            super::validate_box(Some(&s.box_css))?;
            if s.hint.len() > 2048 {
                return Err("MD-N2: invalid range hint".into());
            }
        }
        Ok(())
    }
}
