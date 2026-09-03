use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentMode {
    Browser,
    Computer,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum InputTarget {
    Desktop,
    BrowserTab(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentActionState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentPointerButton {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnvironmentActionArguments {
    PointerMove {
        x: u32,
        y: u32,
        viewport_revision: u64,
    },
    PointerDrag {
        from_x: u32,
        from_y: u32,
        to_x: u32,
        to_y: u32,
        button: EnvironmentPointerButton,
        viewport_revision: u64,
    },
    PointerScroll {
        x: u32,
        y: u32,
        horizontal_steps: i16,
        vertical_steps: i16,
        viewport_revision: u64,
    },
    KeyboardText {
        utf8_byte_count: u32,
        character_count: u32,
    },
    KeyboardKey {
        repeat: u16,
    },
    PointerClick {
        x: u32,
        y: u32,
        button: EnvironmentPointerButton,
        click_count: u8,
        viewport_revision: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentActionTerminal {
    Completed,
    Failed,
    Cancelled,
}

impl From<EnvironmentActionTerminal> for EnvironmentActionState {
    fn from(terminal: EnvironmentActionTerminal) -> Self {
        match terminal {
            EnvironmentActionTerminal::Completed => Self::Completed,
            EnvironmentActionTerminal::Failed => Self::Failed,
            EnvironmentActionTerminal::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentAction {
    pub action_id: String,
    #[serde(default)]
    pub sequence: u64,
    pub idempotency_key: Option<String>,
    pub actor_id: String,
    pub runtime_generation: u64,
    pub mode: EnvironmentMode,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<EnvironmentActionArguments>,
    pub targets: Vec<InputTarget>,
    pub state: EnvironmentActionState,
    #[serde(default)]
    pub cancellation_requested: bool,
    #[serde(default)]
    pub submitted_at_ms: u64,
    #[serde(default)]
    pub started_at_ms: Option<u64>,
    #[serde(default)]
    pub finished_at_ms: Option<u64>,
    #[serde(default)]
    pub outcome: Option<EnvironmentActionOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentActionHistoryPage {
    pub actions: Vec<EnvironmentAction>,
    pub next_before_sequence: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EnvironmentActionOutcome {
    Completed,
    Failed {
        code: EnvironmentActionFailureCode,
    },
    Cancelled {
        reason: EnvironmentActionCancellationReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentActionFailureCode {
    ControllerFailure,
    ProcessLost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentActionCancellationReason {
    Requested,
    HumanTakeover,
    ControllerCancellation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentActionRequest {
    pub(crate) idempotency_key: Option<String>,
    idempotency_fingerprint: Option<[u8; 32]>,
    pub(crate) arguments: Option<EnvironmentActionArguments>,
    pub(crate) actor_id: String,
    pub(crate) runtime_generation: u64,
    pub(crate) mode: EnvironmentMode,
    pub(crate) kind: String,
    pub(crate) mutates: bool,
    pub(crate) targets: Vec<InputTarget>,
    pub(crate) tab_preconditions: Vec<(String, u64)>,
}

impl EnvironmentActionRequest {
    pub fn browser_observation(
        actor_id: impl Into<String>,
        runtime_generation: u64,
        kind: impl Into<String>,
        tab_id: impl Into<String>,
        document_revision: u64,
    ) -> Self {
        let tab_id = tab_id.into();
        Self {
            idempotency_key: None,
            idempotency_fingerprint: None,
            arguments: None,
            actor_id: actor_id.into(),
            runtime_generation,
            mode: EnvironmentMode::Browser,
            kind: kind.into(),
            mutates: false,
            targets: vec![InputTarget::BrowserTab(tab_id.clone())],
            tab_preconditions: vec![(tab_id, document_revision)],
        }
    }

    pub fn browser_mutation(
        actor_id: impl Into<String>,
        runtime_generation: u64,
        kind: impl Into<String>,
        tab_id: impl Into<String>,
        document_revision: u64,
    ) -> Self {
        let mut request = Self::browser_observation(
            actor_id,
            runtime_generation,
            kind,
            tab_id,
            document_revision,
        );
        request.mutates = true;
        request
    }

    pub fn computer_mutation(
        actor_id: impl Into<String>,
        runtime_generation: u64,
        kind: impl Into<String>,
        focused_tab_id: Option<&str>,
    ) -> Self {
        let mut targets = BTreeSet::from([InputTarget::Desktop]);
        if let Some(tab_id) = focused_tab_id {
            targets.insert(InputTarget::BrowserTab(tab_id.to_string()));
        }
        Self {
            idempotency_key: None,
            idempotency_fingerprint: None,
            arguments: None,
            actor_id: actor_id.into(),
            runtime_generation,
            mode: EnvironmentMode::Computer,
            kind: kind.into(),
            mutates: true,
            targets: targets.into_iter().collect(),
            tab_preconditions: Vec::new(),
        }
    }

    pub fn with_idempotency_key(mut self, idempotency_key: impl Into<String>) -> Self {
        self.idempotency_key = Some(idempotency_key.into());
        self
    }

    pub(crate) fn with_arguments(mut self, arguments: EnvironmentActionArguments) -> Self {
        self.arguments = Some(arguments);
        self
    }

    pub(crate) fn with_idempotency_fingerprint(mut self, fingerprint: [u8; 32]) -> Self {
        self.idempotency_fingerprint = Some(fingerprint);
        self
    }

    pub(crate) fn matches_idempotent_operation(&self, other: &Self) -> bool {
        self.idempotency_key == other.idempotency_key
            && self.idempotency_fingerprint == other.idempotency_fingerprint
            && self.arguments == other.arguments
            && self.actor_id == other.actor_id
            && self.mode == other.mode
            && self.kind == other.kind
            && self.mutates == other.mutates
            && (self.arguments.is_some()
                || (self.targets == other.targets
                    && self
                        .tab_preconditions
                        .iter()
                        .map(|(tab_id, _)| tab_id)
                        .eq(other.tab_preconditions.iter().map(|(tab_id, _)| tab_id))))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionAdmission {
    Accepted {
        action_id: String,
    },
    Existing {
        action_id: String,
        state: EnvironmentActionState,
    },
    Queued {
        action_id: String,
        queue_sequence: u64,
    },
    RejectedSaturated {
        capacity: usize,
    },
    RejectedBusy {
        target: InputTarget,
        active_action_id: String,
    },
    RejectedTakeover {
        target: InputTarget,
        human_actor_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ActionCancellationOutcome {
    Cancelled,
    CancellationRequested,
    AlreadyTerminal {
        action_state: EnvironmentActionState,
    },
}
