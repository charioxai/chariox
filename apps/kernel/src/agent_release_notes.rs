//! Bundled landing note and durable delivery receipts.
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;

pub(crate) const LANDING: u32 = 13;
pub(crate) const NOTE: &str = include_str!("../agent-release-notes/13.md");
pub(crate) const TOOL: &str = "chariox.whats_new";

#[derive(Debug)]
pub(crate) struct Delivery {
    cutoff_ms: u64,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    delivered: HashSet<String>,
    prepared_turns: HashMap<String, String>,
}

impl Delivery {
    fn start_kind() -> String {
        format!("agent_release_notes.{LANDING}.started")
    }

    fn delivery_kind() -> String {
        format!("agent_release_notes.{LANDING}.delivered")
    }

    pub(crate) fn open(store: &DurableKernelStateStore) -> Result<Self, DaemonError> {
        let kind = Self::start_kind();
        let cutoff = match store.load_events_by_kind(&kind)?.into_iter().next() {
            Some(start) => start,
            None => store.append_event(kind, None, serde_json::Value::Null)?,
        };
        let delivered = store
            .load_events_by_kind(&Self::delivery_kind())?
            .into_iter()
            .filter_map(|receipt| receipt.subject_id)
            .collect();
        Ok(Self {
            cutoff_ms: cutoff.timestamp_ms,
            state: Mutex::new(State {
                delivered,
                prepared_turns: HashMap::new(),
            }),
        })
    }

    pub(crate) fn prepare_turn(&self, agent_id: &str, created_at_ms: u64, prompt_id: &str) -> bool {
        if created_at_ms >= self.cutoff_ms {
            return false;
        }
        let mut state = self.state.lock().expect("release note receipts");
        if state.delivered.contains(&agent_key(agent_id)) {
            return false;
        }
        state
            .prepared_turns
            .insert(agent_id.into(), prompt_id.into());
        true
    }

    pub(crate) fn acknowledge_turn(
        &self,
        store: &DurableKernelStateStore,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<(), DaemonError> {
        let mut state = self.state.lock().expect("release note receipts");
        if state.prepared_turns.get(agent_id).map(String::as_str) == Some(prompt_id) {
            state.record(store, agent_key(agent_id))?;
            state.prepared_turns.remove(agent_id);
        }
        Ok(())
    }

    pub(crate) fn claim_client(
        &self,
        store: &DurableKernelStateStore,
        session_id: &str,
        created_at_ms: u64,
        client_id: &str,
    ) -> Result<bool, DaemonError> {
        if created_at_ms >= self.cutoff_ms {
            return Ok(false);
        }
        // A client can attach to more than one Room. Include both IDs in its receipt.
        let key = format!("client:{session_id}:{client_id}");
        self.state
            .lock()
            .expect("release note receipts")
            .record(store, key)
    }
}

impl State {
    fn record(
        &mut self,
        store: &DurableKernelStateStore,
        key: String,
    ) -> Result<bool, DaemonError> {
        if self.delivered.contains(&key) {
            return Ok(false);
        }
        store.append_event(
            Delivery::delivery_kind(),
            Some(key.clone()),
            serde_json::Value::Null,
        )?;
        self.delivered.insert(key);
        Ok(true)
    }
}

fn agent_key(agent_id: &str) -> String {
    format!("agent:{agent_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_temp_dir::TestTempDir;

    #[test]
    fn agent_release_notes_receipts_survive_restart() {
        let temp = TestTempDir::new("agent-release-notes");
        let path = temp.join("kernel.db");
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            let delivery = Delivery::open(&store).unwrap();
            let existing = delivery.cutoff_ms - 1;
            assert!(!delivery.prepare_turn("new", delivery.cutoff_ms, "turn"));
            assert!(delivery.prepare_turn("old", existing, "turn"));
            delivery
                .acknowledge_turn(&store, "old", "other-turn")
                .unwrap();
            assert!(delivery.prepare_turn("old", existing, "turn"));
            delivery.acknowledge_turn(&store, "old", "turn").unwrap();
            assert!(!delivery.prepare_turn("old", existing, "next"));
            assert!(delivery
                .claim_client(&store, "room", existing, "client")
                .unwrap());
            assert!(!delivery
                .claim_client(&store, "room", existing, "client")
                .unwrap());
        }
        let store = DurableKernelStateStore::open(path).unwrap();
        let delivery = Delivery::open(&store).unwrap();
        let existing = delivery.cutoff_ms - 1;
        assert!(!delivery.prepare_turn("old", existing, "next"));
        assert!(!delivery
            .claim_client(&store, "room", existing, "client")
            .unwrap());
        assert!(delivery
            .claim_client(&store, "room", existing, "other-client")
            .unwrap());
    }
}
