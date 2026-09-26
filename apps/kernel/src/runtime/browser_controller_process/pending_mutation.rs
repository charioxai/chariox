use std::collections::HashMap;
use std::io::Write;
use std::process::{Child, ChildStdin};
use std::sync::{mpsc, Arc, Mutex, RwLock, TryLockError, Weak};
use std::time::{Duration, Instant};

use super::{
    cancellation, kill_child, pending_responses, BrowserControllerProcessOwnership,
    BrowserControllerProcessState, BrowserControllerProcessStdioBackend,
    BrowserControllerProcessStore, BrowserControllerRpcError, BrowserControllerRpcRequest,
    BrowserControllerRpcResponse, CONTROLLER_RESTARTED_BEFORE_OPERATION,
};

pub(super) struct PendingBrowserMutation {
    request_id: u64,
    cancel_id: u64,
    response: pending_responses::PendingResponse<BrowserControllerRpcResponse>,
    cancellation_response: pending_responses::PendingResponse<BrowserControllerRpcResponse>,
    stdin: Arc<Mutex<ChildStdin>>,
    child: Arc<Mutex<Child>>,
    method: String,
    timeout: Duration,
}

#[derive(Clone, Default)]
pub(super) struct BrowserTabMutationLanes(Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>);

impl BrowserTabMutationLanes {
    fn get(&self, target_id: &str) -> Result<Arc<Mutex<()>>, String> {
        let mut lanes = self
            .0
            .lock()
            .map_err(|_| "browser tab mutation lane registry poisoned")?;
        lanes.retain(|_, lane| lane.strong_count() > 0);
        if let Some(lane) = lanes.get(target_id).and_then(Weak::upgrade) {
            return Ok(lane);
        }
        let lane = Arc::new(Mutex::new(()));
        lanes.insert(target_id.to_string(), Arc::downgrade(&lane));
        Ok(lane)
    }
}

impl BrowserControllerProcessStdioBackend {
    fn begin_cancellable_mutation(
        &mut self,
        method: &str,
        params: &serde_json::Value,
        signal: &cancellation::CancellationSignal,
    ) -> Result<PendingBrowserMutation, String> {
        if signal.requested() {
            signal.confirm_stop();
            return Err("browser action cancelled before dispatch".into());
        }
        let request_id = self.next_request_id;
        let cancel_id = request_id
            .checked_add(1)
            .ok_or("controller request IDs exhausted")?;
        self.next_request_id = cancel_id
            .checked_add(1)
            .ok_or("controller request IDs exhausted")?;
        let process = self
            .process
            .as_ref()
            .ok_or_else(|| "browser controller is not running".to_string())?;
        let response = process.snapshot_responses.register(request_id)?;
        let cancellation_response = process.snapshot_responses.register(cancel_id)?;
        let mut stdin = process
            .stdin
            .lock()
            .map_err(|_| "controller stdin lock poisoned")?;
        serde_json::to_writer(
            &mut *stdin,
            &BrowserControllerRpcRequest {
                id: request_id,
                method,
                params,
            },
        )
        .map_err(|error| format!("failed to encode browser controller `{method}`: {error}"))?;
        stdin
            .write_all(b"\n")
            .and_then(|()| stdin.flush())
            .map_err(|error| format!("failed to send browser controller `{method}`: {error}"))?;
        Ok(PendingBrowserMutation {
            request_id,
            cancel_id,
            response,
            cancellation_response,
            stdin: Arc::clone(&process.stdin),
            child: Arc::clone(&process.child),
            method: method.to_string(),
            timeout: self.timeout,
        })
    }
}

impl PendingBrowserMutation {
    pub(super) fn wait(
        self,
        signal: &cancellation::CancellationSignal,
    ) -> Result<BrowserControllerRpcResponse, String> {
        let started = Instant::now();
        let mut sent = false;
        let mut acknowledged = false;
        let mut terminal = None;
        loop {
            if !sent && signal.requested() {
                let mut stdin = self
                    .stdin
                    .lock()
                    .map_err(|_| "controller stdin lock poisoned")?;
                serde_json::to_writer(
                    &mut *stdin,
                    &serde_json::json!({
                        "id":self.cancel_id,"method":"browser.cancel","params":{"request_id":self.request_id}
                    }),
                )
                .map_err(|error| error.to_string())?;
                stdin
                    .write_all(b"\n")
                    .and_then(|()| stdin.flush())
                    .map_err(|error| error.to_string())?;
                sent = true;
            }
            if sent && !acknowledged {
                match self.cancellation_response.poll(Duration::ZERO) {
                    Ok(response) => {
                        let response = response?;
                        acknowledged = true;
                        if response.ok
                            && response
                                .result
                                .as_ref()
                                .and_then(|value| value.get("accepted"))
                                .and_then(serde_json::Value::as_bool)
                                == Some(true)
                        {
                            signal.confirm_stop();
                        } else {
                            signal.reject_after_stop();
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err("controller exited during cancellation".into())
                    }
                }
            }
            if terminal.is_some() && (!sent || acknowledged) {
                return Ok(terminal.take().unwrap());
            }
            let remaining = self.timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                if signal.requested() {
                    // Fence the exact child that received this request before
                    // confirming a cancellation whose terminal reply is absent.
                    kill_child(&mut self.child.lock().unwrap_or_else(|error| error.into_inner()));
                    signal.confirm_fence();
                    return Ok(BrowserControllerRpcResponse {
                        id: Some(self.request_id),
                        ok: false,
                        result: None,
                        error: Some(BrowserControllerRpcError {
                            code: "browser_action_cancelled".into(),
                            message: "browser controller was fenced after cancellation timed out"
                                .into(),
                        }),
                    });
                }
                return Err(format!(
                    "browser controller `{}` timed out after {}ms",
                    self.method,
                    self.timeout.as_millis()
                ));
            }
            if terminal.is_none() {
                match self.response.poll(remaining.min(Duration::from_millis(20))) {
                    Ok(response) => {
                        let response = response?;
                        if !response.ok
                            && response
                                .error
                                .as_ref()
                                .is_some_and(|error| error.code == "browser_action_cancelled")
                        {
                            signal.confirm_stop();
                        }
                        terminal = Some(response);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(format!("controller exited during `{}`", self.method))
                    }
                }
            } else {
                std::thread::sleep(remaining.min(Duration::from_millis(20)));
            }
        }
    }
}

impl BrowserControllerProcessOwnership<BrowserControllerProcessStdioBackend> {
    pub(super) fn begin_cancellable_tab_mutation(
        &mut self,
        session_id: &str,
        method: &str,
        params: &serde_json::Value,
        signal: &cancellation::CancellationSignal,
    ) -> Result<PendingBrowserMutation, String> {
        self.require_lease(session_id)?;
        let supervisor = &mut self.supervisor;
        let pending_response = supervisor
            .backend
            .process
            .as_ref()
            .map(|process| process.snapshot_responses.is_empty().map(|empty| !empty))
            .transpose()?
            .unwrap_or(false);
        let exited = supervisor.backend.take_exited_process()?.is_some();
        if !pending_response || exited {
            supervisor.ensure_started_without_transparent_restart()?;
        } else if supervisor.recovery_pending
            || supervisor.snapshot.state != BrowserControllerProcessState::Ready
        {
            return Err(CONTROLLER_RESTARTED_BEFORE_OPERATION.into());
        }
        supervisor
            .backend
            .begin_cancellable_mutation(method, params, signal)
    }
}

impl BrowserControllerProcessStore {
    pub(super) fn lock_global_tab_mutation_barrier(
        &self,
    ) -> Result<std::sync::RwLockWriteGuard<'_, ()>, String> {
        self.tab_mutation_barrier
            .write()
            .map_err(|_| "browser tab mutation barrier poisoned".into())
    }

    pub(super) fn lock_tab_mutation_barrier<'a>(
        &'a self,
        signal: &cancellation::CancellationSignal,
    ) -> Result<std::sync::RwLockReadGuard<'a, ()>, String> {
        loop {
            if signal.requested() {
                signal.confirm_stop();
                return Err("browser action cancelled before dispatch".into());
            }
            match self.tab_mutation_barrier.try_read() {
                Ok(guard) => return Ok(guard),
                Err(TryLockError::WouldBlock) => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TryLockError::Poisoned(_)) => {
                    return Err("browser tab mutation barrier poisoned".into());
                }
            }
        }
    }

    pub(super) fn tab_mutation_lane(&self, target_id: &str) -> Result<Arc<Mutex<()>>, String> {
        self.tab_mutation_lanes.get(target_id)
    }

    pub(super) fn lock_tab_mutation_lane<'a>(
        lane: &'a Mutex<()>,
        signal: &cancellation::CancellationSignal,
    ) -> Result<std::sync::MutexGuard<'a, ()>, String> {
        loop {
            if signal.requested() {
                signal.confirm_stop();
                return Err("browser action cancelled before dispatch".into());
            }
            match lane.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(TryLockError::WouldBlock) => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TryLockError::Poisoned(_)) => {
                    return Err("browser tab mutation lane poisoned".into());
                }
            }
        }
    }
}
