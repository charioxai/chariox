//! MP-08/MP-10/MP-11: bounded, zeroizing, exact-turn ephemeral results.
use crate::durable_state::agent_lifecycle::goals;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;
#[derive(Clone, Default)]
pub(in crate::runtime::state) struct AssessmentResults(
    Arc<Mutex<BTreeMap<(String, String), ResultCell>>>,
);
struct ResultCell {
    prompt: String,
    run: String,
    expires: Instant,
    bytes: Zeroizing<Vec<u8>>,
    overflow: bool,
}
impl AssessmentResults {
    pub(in crate::runtime::state) fn append(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
        bytes: &[u8],
    ) {
        let mut cells = self.0.lock().unwrap_or_else(|e| e.into_inner());
        cells.retain(|_, cell| cell.expires > Instant::now());
        let key = (room.to_string(), agent.to_string());
        if !cells.contains_key(&key) && cells.len() >= 128 {
            return;
        }
        let cell = cells.entry(key).or_insert_with(|| ResultCell {
            prompt: prompt.into(),
            run: run.into(),
            expires: Instant::now() + Duration::from_secs(60),
            bytes: Zeroizing::new(Vec::new()),
            overflow: false,
        });
        if cell.prompt != prompt || cell.run != run {
            *cell = ResultCell {
                prompt: prompt.into(),
                run: run.into(),
                expires: Instant::now() + Duration::from_secs(60),
                bytes: Zeroizing::new(Vec::new()),
                overflow: false,
            };
        }
        if cell.overflow {
            return;
        }
        if cell.bytes.len().saturating_add(bytes.len()) > goals::RESULT_LIMIT {
            cell.bytes = Zeroizing::new(Vec::new());
            cell.overflow = true;
        } else {
            cell.bytes.extend_from_slice(bytes);
        }
    }
    pub(in crate::runtime::state) fn take(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
    ) -> Option<Zeroizing<Vec<u8>>> {
        let mut cells = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let key = (room.into(), agent.into());
        let cell = cells.get(&key)?;
        if cell.prompt != prompt || cell.run != run {
            return None;
        }
        let cell = cells.remove(&key)?;
        (!cell.overflow && cell.expires > Instant::now()).then_some(cell.bytes)
    }
}
