use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

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
        loop {
            self.next_session_number = self.next_session_number.wrapping_add(1);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos() as u64)
                .unwrap_or(self.next_session_number);
            let candidate = format!("{:016x}", nanos ^ self.next_session_number.rotate_left(13));
            if !self.sessions.contains_key(&candidate) {
                return candidate;
            }
        }
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

    pub fn remove(&mut self, session_id: &str) -> Option<RuntimeSession> {
        self.sessions.remove(session_id)
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

    pub fn non_ended_sessions(&self) -> impl Iterator<Item = &RuntimeSession> {
        self.sessions
            .values()
            .filter(|session| session.status() != SessionStatus::Ended)
    }
}
