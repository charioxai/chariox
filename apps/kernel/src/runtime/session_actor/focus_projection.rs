use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub(crate) struct FocusedAgentProjection {
    focused_agents: Arc<Mutex<HashMap<String, String>>>,
}

impl FocusedAgentProjection {
    pub(crate) async fn update(&self, session_id: &str, agent_id: Option<&str>) {
        let mut focused_agents = self.focused_agents.lock().await;
        match agent_id {
            Some(agent_id) => {
                focused_agents.insert(session_id.to_string(), agent_id.to_string());
            }
            None => {
                focused_agents.remove(session_id);
            }
        }
    }

    pub(crate) async fn remove(&self, session_id: &str) {
        self.focused_agents.lock().await.remove(session_id);
    }

    pub(crate) async fn focused_agent_id(&self, session_id: &str) -> Option<String> {
        self.focused_agents.lock().await.get(session_id).cloned()
    }
}
