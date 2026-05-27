use serde::{Deserialize, Serialize};

pub const DEFAULT_LOCAL_USER_ID: &str = "local";

pub(in crate::session) fn default_session_owner_user_id() -> String {
    DEFAULT_LOCAL_USER_ID.to_string()
}

pub(in crate::session) fn default_session_members() -> Vec<SessionMember> {
    vec![SessionMember::local()]
}

fn default_session_invite_max_uses() -> Option<u32> {
    Some(1)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationLevel {
    Private,
    Transparent,
    Full,
}

impl Default for CollaborationLevel {
    fn default() -> Self {
        Self::Private
    }
}

impl CollaborationLevel {
    pub fn can_view_agent_trace(self) -> bool {
        matches!(self, Self::Transparent | Self::Full)
    }

    pub fn can_view_agent_parameters(self) -> bool {
        matches!(self, Self::Full)
    }

    pub fn can_prompt_agent_directly(self) -> bool {
        matches!(self, Self::Full)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAgentDefaults {
    pub provider: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<crate::provider::AgentExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_level: Option<crate::provider::AgentPermissionLevel>,
}

impl Default for SessionAgentDefaults {
    fn default() -> Self {
        Self {
            provider: "default".to_string(),
            model: None,
            effort: None,
            account_profile: None,
            execution_mode: None,
            permission_level: None,
        }
    }
}

impl SessionAgentDefaults {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            ..Self::default()
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_effort(mut self, effort: impl Into<String>) -> Self {
        self.effort = Some(effort.into());
        self
    }

    pub fn with_account_profile(mut self, account_profile: impl Into<String>) -> Self {
        self.account_profile = Some(account_profile.into());
        self
    }

    pub fn with_execution_mode(
        mut self,
        execution_mode: crate::provider::AgentExecutionMode,
    ) -> Self {
        self.execution_mode = Some(execution_mode);
        self
    }

    pub fn with_permission_level(
        mut self,
        permission_level: crate::provider::AgentPermissionLevel,
    ) -> Self {
        self.permission_level = Some(permission_level);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub workspace_id: String,
    pub worktree_id: String,
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_defaults: Option<SessionAgentDefaults>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_ref: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default = "default_session_owner_user_id")]
    pub owner_user_id: String,
}

impl CreateSessionRequest {
    pub fn new(workspace_id: impl Into<String>, worktree_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            worktree_id: worktree_id.into(),
            alias: None,
            agent_defaults: None,
            slice_ref: None,
            hidden: false,
            owner_user_id: default_session_owner_user_id(),
        }
    }

    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
        self
    }

    pub fn with_hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    pub fn with_agent_defaults(mut self, agent_defaults: SessionAgentDefaults) -> Self {
        self.agent_defaults = Some(agent_defaults);
        self
    }

    pub fn with_slice_ref(mut self, slice_ref: impl Into<String>) -> Self {
        self.slice_ref = Some(slice_ref.into());
        self
    }

    pub fn with_owner_user_id(mut self, owner_user_id: impl Into<String>) -> Self {
        self.owner_user_id = owner_user_id.into();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMember {
    user_id: String,
    joined_at_ms: u64,
    invited_by_user_id: Option<String>,
    #[serde(default)]
    collaboration_level: CollaborationLevel,
}

impl SessionMember {
    pub fn new(
        user_id: impl Into<String>,
        joined_at_ms: u64,
        invited_by_user_id: Option<String>,
        collaboration_level: CollaborationLevel,
    ) -> Self {
        Self {
            user_id: user_id.into(),
            joined_at_ms,
            invited_by_user_id,
            collaboration_level,
        }
    }

    pub fn local() -> Self {
        Self::new(DEFAULT_LOCAL_USER_ID, 0, None, CollaborationLevel::Full)
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn joined_at_ms(&self) -> u64 {
        self.joined_at_ms
    }

    pub fn invited_by_user_id(&self) -> Option<&str> {
        self.invited_by_user_id.as_deref()
    }

    pub fn collaboration_level(&self) -> CollaborationLevel {
        self.collaboration_level
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInvite {
    invite_id: String,
    session_id: String,
    created_by_user_id: String,
    created_at_ms: u64,
    expires_at_ms: Option<u64>,
    #[serde(default = "default_session_invite_max_uses")]
    max_uses: Option<u32>,
    #[serde(default)]
    used_count: u32,
    revoked_at_ms: Option<u64>,
    #[serde(default)]
    collaboration_level: CollaborationLevel,
}

impl SessionInvite {
    pub fn new(
        invite_id: impl Into<String>,
        session_id: impl Into<String>,
        created_by_user_id: impl Into<String>,
        created_at_ms: u64,
        expires_at_ms: Option<u64>,
        max_uses: Option<u32>,
        collaboration_level: CollaborationLevel,
    ) -> Self {
        Self {
            invite_id: invite_id.into(),
            session_id: session_id.into(),
            created_by_user_id: created_by_user_id.into(),
            created_at_ms,
            expires_at_ms,
            max_uses,
            used_count: 0,
            revoked_at_ms: None,
            collaboration_level,
        }
    }

    pub fn invite_id(&self) -> &str {
        &self.invite_id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn created_by_user_id(&self) -> &str {
        &self.created_by_user_id
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn expires_at_ms(&self) -> Option<u64> {
        self.expires_at_ms
    }

    pub fn max_uses(&self) -> Option<u32> {
        self.max_uses
    }

    pub fn used_count(&self) -> u32 {
        self.used_count
    }

    pub fn revoked_at_ms(&self) -> Option<u64> {
        self.revoked_at_ms
    }

    pub fn collaboration_level(&self) -> CollaborationLevel {
        self.collaboration_level
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked_at_ms.is_some()
    }

    pub fn is_expired(&self, now_ms: u64) -> bool {
        self.expires_at_ms
            .is_some_and(|expires_at_ms| expires_at_ms <= now_ms)
    }

    pub fn is_exhausted(&self) -> bool {
        self.max_uses
            .is_some_and(|max_uses| self.used_count >= max_uses)
    }

    pub fn mark_used(&mut self) {
        self.used_count = self.used_count.saturating_add(1);
    }

    pub fn revoke(&mut self, revoked_at_ms: u64) {
        self.revoked_at_ms = Some(revoked_at_ms);
    }
}
