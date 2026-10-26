//! MP-08/MP-10: unchanged-viewport preflight observes the browser without
//! draining unrelated tab input. Startup, recovery and resize retain barriers.

use super::*;

impl BrowserControllerProcessStdioBackend {
    fn begin_reconciliation_read(
        &mut self,
        viewport: &CanonicalViewport,
        browser_bar_visible: bool,
    ) -> Result<pending_responses::PendingResponse<BrowserControllerRpcResponse>, String> {
        // Same parameters as the barrier path (Room browser bar, App panel width).
        self.begin_observation_request(
            "browser.reconcile",
            browser_reconcile_params(viewport, browser_bar_visible),
        )
    }

    pub(super) fn begin_observation_request(
        &mut self,
        method: &'static str,
        params: serde_json::Value,
    ) -> Result<pending_responses::PendingResponse<BrowserControllerRpcResponse>, String> {
        let request_id = self.next_request_id;
        self.next_request_id = request_id
            .checked_add(1)
            .ok_or("controller request IDs exhausted")?;
        let process = self
            .process
            .as_ref()
            .ok_or("browser controller is not running")?;
        let pending = process.pending_responses.register(
            request_id,
            format!("browser controller exited during `{method}`"),
        )?;
        let mut stdin = process
            .stdin
            .lock()
            .map_err(|_| "controller stdin lock poisoned")?;
        request_wire::write_line(
            &mut *stdin,
            &BrowserControllerRpcRequest {
                id: request_id,
                protected_values: self
                    .protected_values
                    .values()
                    .flatten()
                    .map(|value| value.as_str())
                    .collect(),
                method,
                params: &params,
            },
        )
        .map_err(|error| format!("failed to encode browser controller `{method}`: {error}"))?;
        Ok(pending)
    }
}

impl BrowserControllerProcessStore {
    pub(super) fn acquire_busy_lease(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserControllerProcessSnapshot>, String> {
        let _read = self
            .tab_mutation_barrier
            .read()
            .map_err(|_| "browser tab mutation barrier poisoned")?;
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned")?;
        self.authorize()?;
        if ownership.require_lease(session_id).is_err() {
            return Ok(None);
        }
        let supervisor = &mut ownership.supervisor;
        let pending = supervisor
            .backend
            .process
            .as_ref()
            .map(|process| process.pending_responses.is_empty().map(|empty| !empty))
            .transpose()?
            .unwrap_or(false);
        if pending
            && supervisor.backend.take_exited_process()?.is_none()
            && supervisor.snapshot.state == BrowserControllerProcessState::Ready
            && !supervisor.recovery_pending
        {
            return Ok(Some(supervisor.snapshot.clone()));
        }
        Ok(None)
    }

    pub(super) fn reconcile_browser_observation(
        &self,
        session_id: &str,
        viewport: &CanonicalViewport,
        browser_bar_visible: bool,
    ) -> Result<Option<BrowserControllerReconciliation>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        // Mutations also hold a read guard. Only lifecycle/resize needs a writer.
        let read = self
            .tab_mutation_barrier
            .read()
            .map_err(|_| "browser tab mutation barrier poisoned")?;
        let pending = {
            let mut ownership = ownership
                .lock()
                .map_err(|_| "browser controller supervisor lock poisoned")?;
            self.authorize()?;
            ownership.require_lease(session_id)?;
            let supervisor = &mut ownership.supervisor;
            let exited = supervisor.backend.take_exited_process()?.is_some();
            if !exited
                && !supervisor.recovery_pending
                && supervisor.snapshot.state == BrowserControllerProcessState::Ready
                && supervisor.reconciled_viewport.as_ref() == Some(viewport)
                && supervisor.reconciled_browser_bar_visible == browser_bar_visible
            {
                Some((
                    supervisor
                        .backend
                        .begin_reconciliation_read(viewport, browser_bar_visible)?,
                    supervisor.snapshot.clone(),
                    supervisor.backend.timeout,
                ))
            } else {
                None
            }
        };
        if let Some((pending, process, timeout)) = pending {
            let browser = pending
                .wait(timeout)?
                .into_result::<BrowserControllerBrowserSnapshot>("browser.reconcile")?;
            browser.validate(viewport)?;
            return Ok(Some(BrowserControllerReconciliation { process, browser }));
        }
        drop(read);
        let _barrier = self.lock_global_tab_mutation_barrier()?;
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned")?;
        self.authorize()?;
        ownership
            .reconcile_browser(session_id, viewport, browser_bar_visible)
            .map(Some)
    }
}

impl BrowserControllerProcessStore {
    pub(super) fn wait_for_browser_observation(
        &self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        wait: &BrowserCompatibilityWait,
        timeout_ms: u64,
    ) -> Result<Option<BrowserControllerCompatibilityWaitResult>, String> {
        wait.validate(timeout_ms)?;
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let (pending, timeout) = {
            let mut ownership = ownership
                .lock()
                .map_err(|_| "browser controller supervisor lock poisoned")?;
            self.authorize()?;
            ownership.require_lease(session_id)?;
            let supervisor = &mut ownership.supervisor;
            supervisor.prepare_unlocked_request()?;
            (
                supervisor.backend.begin_observation_request(
                    "browser.wait",
                    serde_json::json!({
                        "target_id": target_id, "document_id": document_id, "kind": wait.kind(),
                        "selector": wait.selector(), "timeout_ms": timeout_ms,
                    }),
                )?,
                supervisor.backend.timeout,
            )
        };
        let result = pending
            .wait(timeout)?
            .into_result::<BrowserControllerCompatibilityWaitResult>("browser.wait")?;
        result.validate(target_id, document_id, wait)?;
        Ok(Some(result))
    }
}
