use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    Idle,
    Working,
    Focused,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridPosition {
    pub row: u32,
    pub col: u32,
    pub row_span: u32,
    pub col_span: u32,
}

impl GridPosition {
    pub fn new(row: u32, col: u32, row_span: u32, col_span: u32) -> Self {
        Self {
            row,
            col,
            row_span,
            col_span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInstance {
    id: String,
    agent_ref: String,
    session_id: String,
    alias: Option<String>,
    provider: String,
    model: Option<String>,
    effort: Option<String>,
    worktree_id: Option<String>,
    state: AgentState,
    is_processing: bool,
    position: GridPosition,
    created_at_ms: u64,
    last_activity_at_ms: u64,
}

impl AgentInstance {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        agent_ref: impl Into<String>,
        session_id: impl Into<String>,
        alias: Option<String>,
        provider: impl Into<String>,
        model: Option<String>,
        effort: Option<String>,
        worktree_id: Option<String>,
        position: GridPosition,
    ) -> Self {
        let now = crate::session::unix_epoch_ms();
        Self {
            id: id.into(),
            agent_ref: agent_ref.into(),
            session_id: session_id.into(),
            alias,
            provider: provider.into(),
            model,
            effort,
            worktree_id,
            state: AgentState::Idle,
            is_processing: false,
            position,
            created_at_ms: now,
            last_activity_at_ms: now,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn agent_ref(&self) -> &str {
        &self.agent_ref
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn effort(&self) -> Option<&str> {
        self.effort.as_deref()
    }

    pub fn worktree_id(&self) -> Option<&str> {
        self.worktree_id.as_deref()
    }

    pub fn state(&self) -> AgentState {
        self.state
    }

    pub fn is_processing(&self) -> bool {
        self.is_processing
    }

    pub fn position(&self) -> &GridPosition {
        &self.position
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn last_activity_at_ms(&self) -> u64 {
        self.last_activity_at_ms
    }

    pub fn set_state(&mut self, state: AgentState) {
        self.state = state;
        self.last_activity_at_ms = crate::session::unix_epoch_ms();
    }

    pub fn set_processing(&mut self, is_processing: bool) {
        self.is_processing = is_processing;
        self.last_activity_at_ms = crate::session::unix_epoch_ms();
    }

    pub fn set_position(&mut self, position: GridPosition) {
        self.position = position;
    }

    pub fn set_alias(&mut self, alias: Option<String>) {
        self.alias = alias;
    }

    pub fn set_model(&mut self, model: Option<String>) {
        self.model = model;
    }

    pub fn set_provider(&mut self, provider: impl Into<String>) {
        self.provider = provider.into();
    }

    pub fn set_effort(&mut self, effort: Option<String>) {
        self.effort = effort;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAgentRequest {
    pub session_id: String,
    pub alias: Option<String>,
    pub provider: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub worktree_id: Option<String>,
}

impl CreateAgentRequest {
    pub fn new(session_id: impl Into<String>, provider: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            alias: None,
            provider: provider.into(),
            model: None,
            effort: None,
            worktree_id: None,
        }
    }

    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_effort(mut self, effort: impl Into<String>) -> Self {
        self.effort = Some(effort.into());
        self
    }

    pub fn with_worktree(mut self, worktree_id: impl Into<String>) -> Self {
        self.worktree_id = Some(worktree_id.into());
        self
    }
}

/// Calculates grid layout for agents based on count.
/// Layout progression:
/// - 1 agent: full screen (2x2)
/// - 2 agents: split vertically (1x2)
/// - 3 agents: split horizontally, leave 1 empty (2x2 with 1 slot)
/// - 4 agents: fill 2x2 grid
/// - 5+ agents: expand the two-row grid horizontally as needed
pub fn calculate_agent_layout(agent_count: usize) -> Vec<GridPosition> {
    match agent_count {
        1 => vec![GridPosition::new(0, 0, 2, 2)],
        2 => vec![GridPosition::new(0, 0, 2, 1), GridPosition::new(0, 1, 2, 1)],
        count => {
            let column_count = count.div_ceil(2);
            let mut positions = Vec::with_capacity(count);
            for index in 0..count {
                let row = if index < column_count { 0 } else { 1 };
                let col = if index < column_count {
                    index
                } else {
                    index - column_count
                };
                positions.push(GridPosition::new(row as u32, col as u32, 1, 1));
            }
            positions
        }
    }
}

/// Recalculate positions for all agents after adding/removing
pub fn recalculate_positions(agents: &mut [AgentInstance]) {
    let positions = calculate_agent_layout(agents.len());
    for (i, agent) in agents.iter_mut().enumerate() {
        if let Some(position) = positions.get(i) {
            agent.set_position(position.clone());
        }
    }
}

/// Generate a git-like agent reference (8-char hex)
pub fn generate_agent_ref() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let hex_chars: Vec<char> = (0..8)
        .map(|_| rng.gen_range(0..16))
        .map(|n| std::char::from_digit(n, 16).unwrap())
        .collect();
    hex_chars.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::{calculate_agent_layout, GridPosition};

    #[test]
    fn calculate_agent_layout_expands_past_six_agents() {
        assert_eq!(
            calculate_agent_layout(7),
            vec![
                GridPosition::new(0, 0, 1, 1),
                GridPosition::new(0, 1, 1, 1),
                GridPosition::new(0, 2, 1, 1),
                GridPosition::new(0, 3, 1, 1),
                GridPosition::new(1, 0, 1, 1),
                GridPosition::new(1, 1, 1, 1),
                GridPosition::new(1, 2, 1, 1),
            ]
        );
    }
}
