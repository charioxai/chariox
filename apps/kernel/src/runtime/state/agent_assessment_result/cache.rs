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
    sealed: bool,
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
            sealed: true,
        });
        if cell.prompt != prompt || cell.run != run {
            *cell = ResultCell {
                prompt: prompt.into(),
                run: run.into(),
                expires: Instant::now() + Duration::from_secs(60),
                bytes: Zeroizing::new(Vec::new()),
                overflow: false,
                sealed: true,
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
        if !cell.sealed {
            return None;
        }
        let cell = cells.remove(&key)?;
        (!cell.overflow && cell.expires > Instant::now()).then_some(cell.bytes)
    }
}

impl AssessmentResults {
    pub(in crate::runtime::state) fn begin_work(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
    ) {
        let mut cells = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let key = (room.into(), agent.into());
        cells.retain(|k, c| k == &key || c.expires > Instant::now());
        if cells
            .get(&key)
            .is_some_and(|c| c.prompt == prompt && c.run == run)
        {
            return;
        }
        if !cells.contains_key(&key) && cells.len() >= 128 {
            return;
        }
        cells.insert(
            key,
            ResultCell {
                prompt: prompt.into(),
                run: run.into(),
                expires: Instant::now() + Duration::from_secs(3600),
                bytes: Zeroizing::new(Vec::new()),
                overflow: false,
                sealed: false,
            },
        );
    }
    pub(in crate::runtime::state) fn append_work(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
        bytes: &[u8],
    ) {
        let mut cells = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(cell) = cells.get_mut(&(room.into(), agent.into())) else {
            return;
        };
        if cell.prompt != prompt || cell.run != run || cell.sealed || cell.overflow {
            return;
        }
        if cell.expires <= Instant::now()
            || cell.bytes.len().saturating_add(bytes.len()) > goals::RESULT_LIMIT
        {
            cell.bytes = Zeroizing::new(Vec::new());
            cell.overflow = true;
            return;
        }
        cell.bytes.extend_from_slice(bytes);
        cell.expires = Instant::now() + Duration::from_secs(3600);
    }
    pub(in crate::runtime::state) fn seal_work(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
    ) {
        let mut cells = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = cells
            .get_mut(&(room.into(), agent.into()))
            .filter(|c| c.prompt == prompt && c.run == run && !c.sealed)
        {
            if c.expires <= Instant::now() {
                c.bytes = Zeroizing::new(Vec::new());
                c.overflow = true;
            }
            c.sealed = true;
            c.expires = Instant::now() + Duration::from_secs(60);
        }
    }
    pub(in crate::runtime::state) fn discard(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
    ) {
        let mut cells = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let key = (room.into(), agent.into());
        if cells
            .get(&key)
            .is_some_and(|c| c.prompt == prompt && c.run == run)
        {
            cells.remove(&key);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp11_work_result_requires_begin_seal_exact_binding_and_never_reconstructs_expired_prefix() {
        let c = AssessmentResults::default();
        c.append_work("r", "a", "p", "run", b"late");
        assert!(c.take("r", "a", "p", "run").is_none());
        c.begin_work("r", "a", "p", "run");
        c.append_work("r", "a", "p", "run", b"first");
        c.begin_work("r", "a", "p", "run");
        assert!(c.take("r", "a", "p", "run").is_none());
        c.append_work("r", "a", "p", "foreign", b"foreign");
        c.append_work("r", "a", "p", "run", b"last");
        c.seal_work("r", "a", "p", "run");
        assert_eq!(
            c.take("r", "a", "p", "run").unwrap().as_slice(),
            b"firstlast"
        );
        assert!(c.take("r", "a", "p", "run").is_none());
        for expired in [false, true] {
            c.begin_work("r", "a", "p2", "run");
            if expired {
                c.0.lock()
                    .unwrap()
                    .get_mut(&("r".into(), "a".into()))
                    .unwrap()
                    .expires = Instant::now() - Duration::from_secs(1);
            } else {
                c.append_work("r", "a", "p2", "run", &vec![0; goals::RESULT_LIMIT + 1]);
            }
            c.append_work("r", "a", "p2", "run", b"cannot repair prefix");
            c.seal_work("r", "a", "p2", "run");
            assert!(c.take("r", "a", "p2", "run").is_none());
        }
    }
}
