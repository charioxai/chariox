use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub(super) struct CancellationSignal {
    requested: AtomicBool,
    stopped: AtomicBool,
    fenced: AtomicBool,
}

impl CancellationSignal {
    pub(super) fn requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
    pub(super) fn confirm_stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }
    pub(super) fn confirm_fence(&self) {
        self.fenced.store(true, Ordering::Release);
        self.confirm_stop();
    }
    fn fenced(&self) -> bool {
        self.fenced.load(Ordering::Acquire)
    }
}

#[derive(Clone, Default)]
pub(super) struct BrowserActionCancellations {
    active: Arc<Mutex<BTreeMap<(String, String), Arc<CancellationSignal>>>>,
}

struct ActiveAction {
    registry: BrowserActionCancellations,
    key: (String, String),
    signal: Arc<CancellationSignal>,
}

impl Drop for ActiveAction {
    fn drop(&mut self) {
        self.registry
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}

impl BrowserActionCancellations {
    fn register(&self, session_id: &str, execution_id: &str) -> Result<ActiveAction, String> {
        if execution_id.len() != 32 || !execution_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("browser action requires a 128-bit execution identity".into());
        }
        let key = (session_id.to_string(), execution_id.to_string());
        let mut active = self
            .active
            .lock()
            .map_err(|_| "browser cancellation registry poisoned")?;
        if active.len() >= 64 || active.contains_key(&key) {
            return Err("browser execution is duplicate or capacity is exhausted".into());
        }
        let signal = Arc::new(CancellationSignal::default());
        active.insert(key.clone(), Arc::clone(&signal));
        Ok(ActiveAction {
            registry: self.clone(),
            key,
            signal,
        })
    }
}

impl BrowserControllerProcessStore {
    pub(crate) fn cancel_browser_action(&self, session_id: &str, execution_id: &str) -> bool {
        let active = self
            .cancellations
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(signal) = active.get(&(session_id.to_string(), execution_id.to_string())) else {
            return false;
        };
        signal.requested.store(true, Ordering::Release);
        true
    }

    pub(crate) fn perform_cancellable_browser_action(
        &self,
        session_id: &str,
        execution_id: &str,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        action: &BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<crate::transport::room_browser_controller::RoomBrowserControllerResult, String>
    {
        use crate::transport::room_browser_controller::RoomBrowserControllerResult as Response;
        let Some(ownership) = &self.ownership else {
            return Ok(Response::Action { result: None });
        };
        let active = self.cancellations.register(session_id, execution_id)?;
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned")?;
        ownership.supervisor.backend.action_cancellation = Some(Arc::clone(&active.signal));
        let result = ownership.perform_browser_action(
            session_id,
            target_id,
            document_id,
            node_ref,
            action,
            timeout_ms,
        );
        ownership.supervisor.backend.action_cancellation = None;
        if active.signal.stopped.load(Ordering::Acquire) {
            let controller_fenced = active.signal.fenced();
            let controller_restarted =
                controller_fenced && ownership.supervisor.ensure_started().is_ok();
            Ok(Response::ActionCancelled {
                controller_fenced,
                controller_restarted,
            })
        } else {
            result.map(|result| Response::Action {
                result: Some(result),
            })
        }
    }
}
