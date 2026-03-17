use std::collections::BTreeMap;

use super::{RuntimeSession, SessionStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStore {
    sessions: BTreeMap<String, RuntimeSession>,
    next_session_number: u64,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            next_session_number: 0,
        }
    }

    pub fn next_session_id(&mut self) -> String {
        self.next_session_number += 1;
        format!("session-{}", self.next_session_number)
    }

    pub fn insert(&mut self, session: RuntimeSession) -> RuntimeSession {
        self.sessions
            .insert(session.id().to_string(), session.clone());
        session
    }

    pub fn get(&self, session_id: &str) -> Option<&RuntimeSession> {
        self.sessions.get(session_id)
    }

    pub fn get_mut(&mut self, session_id: &str) -> Option<&mut RuntimeSession> {
        self.sessions.get_mut(session_id)
    }

    pub fn list(&self) -> Vec<RuntimeSession> {
        self.sessions.values().cloned().collect()
    }

    pub fn active_session_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| session.status() != SessionStatus::Ended)
            .count()
    }
}
