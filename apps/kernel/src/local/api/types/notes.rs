//! MD-N1–N3 / MP-08 / MP-11: owner notes, protocol 443. Payloads never broadcast.
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
    pub selection_id: String,
    pub anchor: NoteAnchor,
    pub box_css: Option<NoteBox>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteReply {
    pub author: String,
    pub comment: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteRecord {
    pub note_id: String,
    pub author: String,
    pub domain: String,
    pub anchor: NoteAnchor,
    pub comment: String,
    pub replies: Vec<NoteReply>,
    pub resolved: bool,
    /// Original quote is immutable even if the page is gone or ambiguous.
    pub anchor_state: String,
    pub box_css: Option<NoteBox>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteSummary {
    pub note_id: String,
    pub window: NoteWindow,
    pub resolved: bool,
    pub anchor_state: String,
    pub updated_at_ms: u64,
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
    Create {
        selection_id: String,
        comment: String,
    },
    List {
        window: NoteWindow,
    },
    Read {
        note_id: String,
    },
    Reply {
        note_id: String,
        comment: String,
    },
    Resolve {
        note_id: String,
    },
    Reanchor {
        note_id: String,
    },
    /// Returns a draft; never queues a prompt.
    Ask {
        note_id: String,
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
    SelectionChanged {
        selection: Option<NoteSelection>,
    },
    NoteChanged {
        note: NoteRecord,
    },
    NotesListed {
        notes: Vec<NoteSummary>,
    },
    PromptDraft {
        note_id: String,
        text: String,
        attachment: crate::session::PromptAttachment,
    },
}
