//! Setup steps of a new Room: what the person asked for, and the progress every client sees.

use serde::{Deserialize, Serialize};

/// What the person asked a new Room to do before work starts, in this order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomSetupRequest {
    #[serde(default)]
    pub git_init: bool,
    #[serde(default)]
    pub new_worktree: bool,
    /// Publish the folder as a private repository named after it, with the machine's own login.
    #[serde(default)]
    pub publish: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomSetupStepId {
    GitInit,
    Worktree,
    Publish,
}

impl RoomSetupStepId {
    pub fn label(self) -> &'static str {
        match self {
            Self::GitInit => "git init",
            Self::Worktree => "worktree",
            Self::Publish => "publish",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomSetupStepState {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomSetupStep {
    pub id: RoomSetupStepId,
    pub state: RoomSetupStepState,
    /// Present on a failed step: what went wrong and what to do next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// The steps of a Room's setup while any is unfinished or failed; absent once all are done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomSetupState {
    pub steps: Vec<RoomSetupStep>,
}

impl RoomSetupState {
    pub fn pending(request: &RoomSetupRequest) -> Option<Self> {
        let ids = [
            (request.git_init, RoomSetupStepId::GitInit),
            (request.new_worktree, RoomSetupStepId::Worktree),
            (request.publish, RoomSetupStepId::Publish),
        ];
        let steps: Vec<_> = ids
            .into_iter()
            .filter(|(wanted, _)| *wanted)
            .map(|(_, id)| RoomSetupStep {
                id,
                state: RoomSetupStepState::Pending,
                message: None,
            })
            .collect();
        (!steps.is_empty()).then_some(Self { steps })
    }

    /// A step is unfinished, or `git_init` or `worktree` failed and was not dismissed. A failed
    /// `publish` never holds work: the checkout is usable without a remote.
    pub fn holds_prompts(&self) -> bool {
        self.steps.iter().any(|step| match step.state {
            RoomSetupStepState::Pending | RoomSetupStepState::Running => true,
            RoomSetupStepState::Failed => step.id != RoomSetupStepId::Publish,
            RoomSetupStepState::Done => false,
        })
    }

    pub fn has_failed(&self) -> bool {
        self.steps
            .iter()
            .any(|step| step.state == RoomSetupStepState::Failed)
    }

    pub fn is_complete(&self) -> bool {
        self.steps
            .iter()
            .all(|step| step.state == RoomSetupStepState::Done)
    }
}
