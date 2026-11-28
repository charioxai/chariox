use serde::{Deserialize, Serialize};

/// The owner terminal's side of a presentation-only exchange.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum OwnerViewRequest {
    Status {},
    Publish {
        generation: String,
        tab_id: String,
        rows: OwnerViewRows,
        /// Local 1016 (additive): what the owner's viewer delivers for each Room Browser pane it shows.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        browser: Option<OwnerViewBrowser>,
        /// Local 1016 (additive): the picture the viewer drew, sent once per `snapshot_wanted` request.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snapshot: Option<OwnerViewSnapshot>,
    },
    End {
        agent_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerViewRows {
    pub navigator: Vec<String>,
    pub settings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerViewWatcher {
    pub agent_id: String,
    pub name: String,
}

/// Delivered display numbers, measured by the viewer from the frames it received and painted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerViewBrowser {
    pub viewer: OwnerViewViewer,
    /// The carrier of the viewer's connection to this kernel, as its Link tool shows it.
    pub path: OwnerViewPath,
    /// The window the rates were counted over.
    pub window_ms: u32,
    pub panes: Vec<OwnerViewPane>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerViewViewer {
    pub kind: OwnerViewViewerKind,
    pub device_pixel_ratio: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerViewViewerKind {
    /// The owner's own Web tab.
    OwnerTab,
    /// A headless browser signed in as the owner, standing in when no owner tab is open.
    Headless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerViewPath {
    Direct,
    Relay,
    Blocked,
    Down,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerViewPane {
    pub tab_id: String,
    /// The pane's paint mode: `video`, `mirror`, `loading`, ...
    pub mode: String,
    pub received_fps: f64,
    /// Video frames presented plus region-video frames presented.
    pub painted_fps: f64,
    pub region_painted_fps: f64,
    pub dropped_fps: f64,
    /// Mirrored-page (still) updates applied.
    pub still_updates_fps: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_to_paint_ms: Option<OwnerViewLatency>,
    pub totals: OwnerViewPaneTotals,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerViewLatency {
    pub p50: f64,
    pub max: f64,
    pub count: u32,
}

/// Counts since the agent's watch began on this viewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerViewPaneTotals {
    pub received: u64,
    pub painted: u64,
    pub dropped: u64,
    pub still_updates: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerViewSnapshot {
    /// The `snapshot_wanted` value this picture answers.
    pub request: u64,
    pub tab_id: String,
    pub source: OwnerViewSnapshotSource,
    pub jpeg_base64: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerViewSnapshotSource {
    /// The pixels of the video canvas the viewer painted.
    ViewerCanvas,
    /// The page picture the viewer holds for a mirrored page.
    DocumentPicture,
    /// The region videos a mirrored page presents, drawn at their places (over the held page picture, when there is one).
    RegionVideo,
}
