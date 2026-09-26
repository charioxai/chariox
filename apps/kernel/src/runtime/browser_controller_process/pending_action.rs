//! One correlated physical Action and cancellation, independent of ownership waits.

use super::*;

pub(super) struct PendingAction {
    request_id: u64,
    cancel_id: u64,
    response: pending_responses::PendingResponse<BrowserControllerRpcResponse>,
    cancellation_response: pending_responses::PendingResponse<BrowserControllerRpcResponse>,
    stdin: Arc<Mutex<ChildStdin>>,
    child: Arc<Mutex<Child>>,
    timeout: Duration,
}

impl StdioOwnership {
    pub(super) fn begin_action(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        action: &BrowserLocatorAction,
        timeout_ms: u64,
        signal: &cancellation::CancellationSignal,
    ) -> Result<PendingAction, String> {
        self.require_lease(session_id)?;
        action.validate()?;
        validate_browser_action_timeout(timeout_ms)?;
        if signal.requested() {
            signal.confirm_stop();
            return Err("browser action cancelled before dispatch".into());
        }
        let supervisor = &mut self.supervisor;
        let pending = supervisor
            .backend
            .process
            .as_ref()
            .map(|process| process.pending_responses.is_empty().map(|empty| !empty))
            .transpose()?
            .unwrap_or(false);
        let exited = supervisor.backend.take_exited_process()?.is_some();
        if !pending || exited {
            supervisor.ensure_started_without_transparent_restart()?;
        } else if supervisor.recovery_pending
            || supervisor.snapshot.state != BrowserControllerProcessState::Ready
        {
            return Err(CONTROLLER_RESTARTED_BEFORE_OPERATION.into());
        }
        let backend = &mut supervisor.backend;
        let request_id = backend.next_request_id;
        let cancel_id = request_id
            .checked_add(1)
            .ok_or("controller request IDs exhausted")?;
        backend.next_request_id = cancel_id
            .checked_add(1)
            .ok_or("controller request IDs exhausted")?;
        let process = backend
            .process
            .as_ref()
            .ok_or("browser controller is not running")?;
        let response = process.pending_responses.register(
            request_id,
            "browser controller exited during `browser.action`",
        )?;
        let cancellation_response = process
            .pending_responses
            .register(cancel_id, "controller exited during cancellation")?;
        let mut stdin = process
            .stdin
            .lock()
            .map_err(|_| "controller stdin lock poisoned")?;
        serde_json::to_writer(
            &mut *stdin,
            &BrowserControllerRpcRequest {
                id: request_id,
                method: "browser.action",
                params: &serde_json::json!({
                    "target_id":target_id,"document_id":document_id,"node_ref":node_ref,
                    "action":action.controller_value(),"timeout_ms":timeout_ms,
                }),
            },
        )
        .map_err(|error| error.to_string())?;
        stdin
            .write_all(b"\n")
            .and_then(|()| stdin.flush())
            .map_err(|error| error.to_string())?;
        Ok(PendingAction {
            request_id,
            cancel_id,
            response,
            cancellation_response,
            stdin: Arc::clone(&process.stdin),
            child: Arc::clone(&process.child),
            timeout: backend
                .timeout
                .saturating_add(Duration::from_millis(timeout_ms)),
        })
    }
}

impl PendingAction {
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
                serde_json::to_writer(&mut *stdin, &serde_json::json!({
                    "id":self.cancel_id,"method":"browser.cancel","params":{"request_id":self.request_id}
                })).map_err(|error| error.to_string())?;
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
                    // Retain the original Child, so a replacement process is never fenced.
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
                    "browser controller `browser.action` timed out after {}ms",
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
                        return Err("controller exited during browser.action".into())
                    }
                }
            } else {
                std::thread::sleep(remaining.min(Duration::from_millis(20)));
            }
        }
    }
}
