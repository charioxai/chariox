use super::action::{EnvironmentAction, EnvironmentActionState, InputTarget};
use super::ownership::{InputOwnership, PendingInputTakeover};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentLifecycle {
    Stopped,
    Starting,
    Ready,
    Degraded,
    Saving,
    Restoring,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalViewport {
    pub css_width: u32,
    pub css_height: u32,
    pub device_scale_factor: u32,
    pub desktop_pixel_width: u32,
    pub desktop_pixel_height: u32,
    pub revision: u64,
    pub last_actor_id: Option<String>,
}

impl CanonicalViewport {
    pub fn new(
        css_width: u32,
        css_height: u32,
        device_scale_factor: u32,
        desktop_pixel_width: u32,
        desktop_pixel_height: u32,
    ) -> Result<Self, EnvironmentError> {
        if [
            css_width,
            css_height,
            device_scale_factor,
            desktop_pixel_width,
            desktop_pixel_height,
        ]
        .contains(&0)
        {
            return Err(EnvironmentError::InvalidViewport);
        }
        Ok(Self {
            css_width,
            css_height,
            device_scale_factor,
            desktop_pixel_width,
            desktop_pixel_height,
            revision: 1,
            last_actor_id: None,
        })
    }

    /// CSS width of the default panel at the right of an App view: at most a
    /// third of the page. App pages lay out in the rest.
    pub fn app_panel_css_width(&self) -> u32 {
        APP_PANEL_CSS_WIDTH.min(self.css_width / 3)
    }

    /// The App page's CSS size and the panel beside it in desktop pixels.
    /// A size is at most half the page; minimized, the panel is a bar at the
    /// bottom and the page keeps the rest.
    pub(crate) fn app_layout(
        &self,
        layout: AppPanelLayout,
        agent_id: Option<String>,
    ) -> ((u32, u32), Option<EnvironmentAppPanel>) {
        let (width, height, scale) = (self.css_width, self.css_height, self.device_scale_factor);
        let Some(placement) = layout.placement else {
            return ((width, height), None);
        };
        let panel = |x: u32, y: u32, w: u32, h: u32| EnvironmentAppPanel {
            x: x.saturating_mul(scale),
            y: y.saturating_mul(scale),
            width: w.saturating_mul(scale),
            height: h.saturating_mul(scale),
            agent_id: agent_id.clone(),
            placement,
            minimized: layout.minimized,
        };
        if layout.minimized {
            let bar = MINIMIZED_CSS_HEIGHT.min(height / 4);
            return (
                (width, height - bar),
                Some(panel(0, height - bar, width, bar)),
            );
        }
        match placement {
            AppPanelPlacement::Right => {
                let size = layout
                    .size
                    .map_or(self.app_panel_css_width(), |size| size.min(width / 2));
                (
                    (width - size, height),
                    Some(panel(width - size, 0, size, height)),
                )
            }
            AppPanelPlacement::Bottom => {
                let size = layout
                    .size
                    .map_or(BOTTOM_PANEL_CSS_HEIGHT.min(height / 3), |size| {
                        size.min(height / 2)
                    });
                (
                    (width, height - size),
                    Some(panel(0, height - size, width, size)),
                )
            }
        }
    }
}

const APP_PANEL_CSS_WIDTH: u32 = 380;
const BOTTOM_PANEL_CSS_HEIGHT: u32 = 260;
const MINIMIZED_CSS_HEIGHT: u32 = 32;

/// Where the agent panel sits beside one App view: the user's choice, else the
/// App's runtime request, else its manifest default (right).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppPanelLayout {
    /// None: the App shows no panel.
    pub placement: Option<AppPanelPlacement>,
    /// CSS pixels: the width at the right or the height at the bottom.
    pub size: Option<u32>,
    pub minimized: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppPanelPlacement {
    #[default]
    Right,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentActorKind {
    Human,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentActorPresence {
    Present,
    Away,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentActorColor {
    Blue,
    Cyan,
    Green,
    Amber,
    Orange,
    Rose,
    Violet,
    Slate,
}

impl Default for EnvironmentActorColor {
    fn default() -> Self {
        Self::Slate
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentActor {
    pub actor_id: String,
    pub kind: EnvironmentActorKind,
    pub display_label: String,
    pub presence: EnvironmentActorPresence,
    #[serde(default)]
    pub presentation_color: EnvironmentActorColor,
}

impl EnvironmentActor {
    pub fn new(
        actor_id: impl Into<String>,
        kind: EnvironmentActorKind,
        display_label: impl Into<String>,
    ) -> Self {
        let actor_id = actor_id.into();
        Self {
            presentation_color: actor_presentation_color(&actor_id),
            actor_id,
            kind,
            display_label: display_label.into(),
            presence: EnvironmentActorPresence::Present,
        }
    }
}

fn actor_presentation_color(actor_id: &str) -> EnvironmentActorColor {
    const COLORS: [EnvironmentActorColor; 8] = [
        EnvironmentActorColor::Blue,
        EnvironmentActorColor::Cyan,
        EnvironmentActorColor::Green,
        EnvironmentActorColor::Amber,
        EnvironmentActorColor::Orange,
        EnvironmentActorColor::Rose,
        EnvironmentActorColor::Violet,
        EnvironmentActorColor::Slate,
    ];
    let hash = actor_id.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    COLORS[hash as usize % COLORS.len()]
}

pub fn human_environment_actor_id(user_id: &str) -> String {
    format!("user:{user_id}")
}

pub fn human_environment_actor_label(user_id: &str) -> &'static str {
    if user_id == crate::session::DEFAULT_LOCAL_USER_ID {
        "Local user"
    } else {
        "Room member"
    }
}

pub fn agent_environment_actor_id(agent_id: &str) -> String {
    format!("agent:{agent_id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentComponent {
    BrowserController,
    Browser,
    Desktop,
    Streamer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentComponentHealthState {
    Starting,
    Ready,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentComponentHealth {
    pub component: EnvironmentComponent,
    pub state: EnvironmentComponentHealthState,
    pub diagnostic_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomEnvironmentSnapshot {
    pub session_id: String,
    pub environment_id: String,
    pub runtime_generation: u64,
    pub lifecycle: EnvironmentLifecycle,
    pub health: Vec<EnvironmentComponentHealth>,
    pub viewport: CanonicalViewport,
    pub actors: Vec<EnvironmentActor>,
    #[serde(default)]
    pub pointers: Vec<EnvironmentPointer>,
    pub tabs: Vec<EnvironmentTab>,
    pub focused_tab_id: Option<String>,
    pub actions: Vec<EnvironmentAction>,
    pub input_ownership: Vec<InputOwnership>,
    #[serde(default)]
    pub pending_input_takeovers: Vec<PendingInputTakeover>,
    /// Ordinary Tabs show the browser bar (maximized windows) instead of
    /// covering the desktop like App views (fullscreen, the default).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub browser_bar_visible: bool,
    pub event_cursor: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentPointerPosition {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentPointer {
    pub actor_id: String,
    pub x: u32,
    pub y: u32,
    pub viewport_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentTab {
    pub tab_id: String,
    pub url: String,
    pub title: String,
    pub document_revision: u64,
    pub focused: bool,
    /// Set on an App view Tab. Absent on every other Tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<EnvironmentTabApp>,
}

/// An App view Tab: its installation and the area beside the App page where
/// the trusted terminal draws the private conversation panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentTabApp {
    pub installation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<EnvironmentAppPanel>,
}

/// Desktop pixels of the canonical viewport (the App's window is fullscreen and
/// its page is smaller). The panel shows the session's focus agent, following
/// every focus change; the App never sees it. Minimized, it is a bar at the
/// bottom that the user can restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentAppPanel {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub agent_id: Option<String>,
    #[serde(default)]
    pub placement: AppPanelPlacement,
    #[serde(default)]
    pub minimized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnvironmentTabObservation {
    pub(crate) runtime_target_id: String,
    pub(crate) document_id: String,
    pub(crate) url: String,
    pub(crate) title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnvironmentTabRuntimeBinding {
    pub(crate) runtime_target_id: String,
    pub(crate) document_id: String,
    pub(crate) document_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentError {
    DurableStateUnavailable,
    BrowserImportRecoveryRequired,
    BrowserImportRecoveryStateUnavailable,
    InvalidViewport,
    EnvironmentAlreadyExists {
        session_id: String,
        environment_id: String,
    },
    EnvironmentNotFound {
        session_id: String,
    },
    RoomNotFound {
        session_id: String,
    },
    InvalidLifecycleTransition {
        from: EnvironmentLifecycle,
        to: EnvironmentLifecycle,
    },
    StaleRuntimeGeneration {
        expected: u64,
        actual: u64,
    },
    UnknownTab {
        tab_id: String,
    },
    StaleDocumentRevision {
        tab_id: String,
        expected: u64,
        actual: u64,
    },
    StructuredObservationUnavailable {
        tab_id: String,
    },
    StaleElementReference {
        reference_id: String,
    },
    UnknownActor {
        actor_id: String,
    },
    StaleViewportRevision {
        expected: u64,
        actual: u64,
    },
    EnvironmentNotReady {
        lifecycle: EnvironmentLifecycle,
    },
    UnknownAction {
        action_id: String,
    },
    HumanActorRequired {
        actor_id: String,
    },
    InputOwnedByAnotherActor {
        target: InputTarget,
        actor_id: String,
    },
    InputNotOwned {
        target: InputTarget,
    },
    InputTakeoverRequired {
        actor_id: String,
    },
    PointerOutOfBounds {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    InvalidClickCount {
        click_count: u8,
    },
    InvalidScrollSteps {
        horizontal_steps: i16,
        vertical_steps: i16,
        max_steps: u16,
    },
    InvalidKeyboardText {
        utf8_byte_count: usize,
        max_utf8_bytes: usize,
    },
    InvalidKeyboardKey,
    InvalidTargetAction,
    InvalidHoldDuration {
        duration_ms: u32,
        max_duration_ms: u32,
    },
    InvalidKeyboardRepeat {
        repeat: u16,
        max_repeat: u16,
    },
    InvalidClipboardText {
        utf8_byte_count: usize,
        max_utf8_bytes: usize,
    },
    InvalidIdempotencyKey,
    InvalidEventCapacity,
    IdempotencyConflict {
        idempotency_key: String,
    },
    ActionAlreadyTerminal {
        action_id: String,
        state: EnvironmentActionState,
    },
    ActionNotRunning {
        action_id: String,
        state: EnvironmentActionState,
    },
    ActionCancellationForbidden {
        actor_id: String,
        action_id: String,
    },
    ActorKindConflict {
        actor_id: String,
    },
}

impl EnvironmentError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::DurableStateUnavailable => "environment_durable_state_unavailable",
            Self::BrowserImportRecoveryRequired => "environment_browser_import_recovery_required",
            Self::BrowserImportRecoveryStateUnavailable => {
                "environment_browser_import_recovery_state_unavailable"
            }
            Self::InvalidViewport => "environment_invalid_viewport",
            Self::EnvironmentAlreadyExists { .. } => "environment_already_exists",
            Self::EnvironmentNotFound { .. } => "environment_not_found",
            Self::RoomNotFound { .. } => "room_not_found",
            Self::InvalidLifecycleTransition { .. } => "environment_invalid_lifecycle_transition",
            Self::StaleRuntimeGeneration { .. } => "environment_stale_runtime_generation",
            Self::UnknownTab { .. } => "environment_unknown_tab",
            Self::StaleDocumentRevision { .. } => "environment_stale_document_revision",
            Self::StructuredObservationUnavailable { .. } => {
                "environment_structured_observation_unavailable"
            }
            Self::StaleElementReference { .. } => "environment_stale_element_reference",
            Self::UnknownActor { .. } => "environment_unknown_actor",
            Self::StaleViewportRevision { .. } => "environment_stale_viewport_revision",
            Self::EnvironmentNotReady { .. } => "environment_not_ready",
            Self::UnknownAction { .. } => "environment_unknown_action",
            Self::HumanActorRequired { .. } => "environment_human_actor_required",
            Self::InputOwnedByAnotherActor { .. } => "environment_input_owned",
            Self::InputNotOwned { .. } => "environment_input_not_owned",
            Self::InputTakeoverRequired { .. } => "environment_input_takeover_required",
            Self::PointerOutOfBounds { .. } => "environment_pointer_out_of_bounds",
            Self::InvalidClickCount { .. } => "environment_invalid_click_count",
            Self::InvalidScrollSteps { .. } => "environment_invalid_scroll_steps",
            Self::InvalidKeyboardText { .. } => "environment_invalid_keyboard_text",
            Self::InvalidHoldDuration { .. } => "environment_invalid_hold_duration",
            Self::InvalidKeyboardKey => "environment_invalid_keyboard_key",
            Self::InvalidTargetAction => "environment_invalid_target_action",
            Self::InvalidKeyboardRepeat { .. } => "environment_invalid_keyboard_repeat",
            Self::InvalidClipboardText { .. } => "environment_invalid_clipboard_text",
            Self::InvalidIdempotencyKey => "environment_invalid_idempotency_key",
            Self::InvalidEventCapacity => "environment_invalid_event_capacity",
            Self::IdempotencyConflict { .. } => "environment_idempotency_conflict",
            Self::ActionAlreadyTerminal { .. } => "environment_action_terminal",
            Self::ActionNotRunning { .. } => "environment_action_not_running",
            Self::ActionCancellationForbidden { .. } => "environment_action_cancellation_forbidden",
            Self::ActorKindConflict { .. } => "environment_actor_kind_conflict",
        }
    }
}
