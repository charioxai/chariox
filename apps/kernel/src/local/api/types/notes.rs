//! Selection capture for prompt notes. Notes themselves travel as prompt attachments.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NoteWindow {
    KernelBrowser {
        tab_id: String,
        generation: u64,
    },
    RoomBrowser {
        session_id: String,
        tab_id: String,
    },
    Panel {
        window_id: String,
    },
    Terminal {
        session_id: String,
        window_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteTextQuote {
    pub exact: String,
    pub prefix: String,
    pub suffix: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteAnchor {
    pub window: NoteWindow,
    pub url: Option<String>,
    pub document_id: Option<String>,
    /// DOM range, message/node identity or scrollback range; only a hint.
    pub hint: Option<String>,
    pub quote: NoteTextQuote,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteSelection {
    pub anchor: NoteAnchor,
    pub box_css: Option<NoteBox>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum NoteCommand {
    CaptureSelection {
        window: NoteWindow,
    },
    /// Native panels only. Streamed surfaces must use controller capture.
    ReportSelection {
        anchor: NoteAnchor,
        box_css: Option<NoteBox>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotesRequest {
    pub command: NoteCommand,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum NoteResult {
    SelectionChanged { selection: Option<NoteSelection> },
}
