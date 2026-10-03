use super::*;

pub(crate) const ENVIRONMENT_USE_ADMISSION_EXPIRED: &str =
    "Room Environment operation admission deadline expired before dispatch";

impl SliceStore {
    /// FIFO admission for controller routes and viewer reads. Admitted uses
    /// share the slice, but a queued use waits for a controller route already
    /// in flight. Only controller contention is retryable; lifecycle,
    /// quarantine and scope errors remain refusals. Waiting never dispatches
    /// or retries the caller's command.
    pub(crate) async fn queue_environment_use(
        &self,
        slice_ref: &str,
        session_id: Option<&str>,
        operation: &'static str,
    ) -> Result<SliceEnvironmentUseGuard, DaemonError> {
        self.queue_environment_use_until(
            slice_ref,
            session_id,
            operation,
            tokio::time::Instant::now() + ENVIRONMENT_USE_ADMISSION_TIMEOUT,
        )
        .await
    }

    pub(crate) async fn queue_environment_use_until(
        &self,
        slice_ref: &str,
        session_id: Option<&str>,
        operation: &'static str,
        deadline: tokio::time::Instant,
    ) -> Result<SliceEnvironmentUseGuard, DaemonError> {
        let slice = self.resolve(slice_ref)?;
        let queue = {
            let mut state = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state
                .environment_use_queues
                .entry(slice.id.clone())
                .or_default()
                .clone()
        };
        let timed_out = || DaemonError::LocalTransport {
            operation,
            message: ENVIRONMENT_USE_ADMISSION_EXPIRED.to_string(),
        };
        let queue = tokio::time::timeout_at(deadline, queue.lock_owned())
            .await
            .map_err(|_| timed_out())?;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(timed_out());
            }
            self.check_shared_environment_use(
                &slice.id,
                session_id,
                CONTROLLER_ROUTE_OPERATION,
                operation,
            )?;
            // Normal queued callers serialize on the FIFO mutex. This also
            // waits for an already-admitted synchronous route.
            let route_in_flight = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .environment_route_uses
                .contains_key(&slice.id);
            if !route_in_flight {
                match self.guard_environment_use(&slice.id, session_id, operation) {
                    Ok(guard) => {
                        return Ok(SliceEnvironmentUseGuard {
                            _operation: guard,
                            _queue: Some(queue),
                        })
                    }
                    Err(DaemonError::LocalTransport {
                        operation: "slice.operation",
                        ..
                    }) => {}
                    Err(error) => return Err(error),
                }
            }
            tokio::time::timeout_at(
                deadline,
                tokio::time::sleep(std::time::Duration::from_millis(10)),
            )
            .await
            .map_err(|_| timed_out())?;
        }
    }

    pub(crate) fn guard_environment_use(
        &self,
        slice_ref: &str,
        session_id: Option<&str>,
        operation: &'static str,
    ) -> Result<SliceOperationGuard, DaemonError> {
        let guard = self.begin_operation(slice_ref, operation, true)?;
        // Re-read under the use marker: binding cannot change until admission ends.
        let state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let slice = state
            .records
            .get(&guard.slice_id)
            .ok_or_else(|| access_error("unknown slice"))?;
        // Discovery may have changed identities since the Room was bound. Check
        // direct slice references too, not only alias lookups.
        if has_shared_worker(slice, &state) {
            return Err(access_error(
                "slice worker reference is shared by another slice",
            ));
        }
        require_environment_session(slice, session_id)?;
        Ok(guard)
    }

    /// Admit a command that may overlap other controller commands: it checks
    /// the same binding as `guard_environment_use` without taking the slice's
    /// operation slot. It still yields to lifecycle operations (start, stop,
    /// restore) and to a quarantining restore.
    pub(crate) fn check_shared_environment_use(
        &self,
        slice_id: &str,
        session_id: Option<&str>,
        shares_with: &'static str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let slice = state
            .records
            .get(slice_id)
            .ok_or_else(|| access_error("unknown slice"))?;
        if let Some(pending) = super::pending_backup_restore_for_slice(&state, slice_id) {
            return Err(super::unresolved_backup_restore_error(
                operation,
                &slice.name,
                &pending.id,
            ));
        }
        if let Some(existing) = state
            .active_operations
            .get(slice_id)
            .filter(|existing| existing.as_str() != shares_with)
        {
            return Err(DaemonError::LocalTransport {
                operation: "slice.operation",
                message: format!(
                    "slice `{}` already has an active `{existing}` operation",
                    slice.name
                ),
            });
        }
        if has_shared_worker(slice, &state) {
            return Err(access_error(
                "slice worker reference is shared by another slice",
            ));
        }
        require_environment_session(slice, session_id)
    }

    /// Sessions whose Room is bound to a slice.
    pub(crate) fn environment_sessions(&self) -> Vec<String> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .records
            .values()
            .filter_map(|slice| slice.environment_session_id.clone())
            .collect()
    }

    pub(crate) fn environment_slice(&self, session_id: &str) -> Option<SliceRecord> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .records
            .values()
            .find(|slice| slice.environment_session_id.as_deref() == Some(session_id))
            .cloned()
    }

    /// Commit the reservation before publishing it. Claims across Room lanes
    /// share this lock, so a physical profile can never have two owners.
    pub(crate) fn bind_environment(
        &self,
        session_id: &str,
        slice_ref: &str,
        now_ms: u64,
        persist: impl FnOnce(&SliceRecord) -> Result<(), DaemonError>,
    ) -> Result<SliceRecord, DaemonError> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut matches = state
            .records
            .values()
            .filter(|slice| slice.id == slice_ref.trim() || slice.name == slice_ref.trim());
        let slice = matches
            .next()
            .ok_or_else(|| binding_error("unknown slice"))?;
        if matches.next().is_some() {
            return Err(binding_error("ambiguous slice reference"));
        }
        if slice.display_mode != super::super::SliceDisplayMode::Headed {
            return Err(binding_error("Room Environment requires a headed slice"));
        }
        if has_shared_worker(slice, &state) {
            return Err(binding_error(
                "slice worker reference is shared by another slice",
            ));
        }
        if state.records.values().any(|other| {
            other.environment_session_id.as_deref() == Some(session_id) && other.id != slice.id
        }) {
            return Err(binding_error(
                "Room already has a different Environment slice",
            ));
        }
        if slice
            .environment_session_id
            .as_deref()
            .is_some_and(|owner| owner != session_id)
            || slice
                .session_id
                .as_deref()
                .is_some_and(|owner| owner != session_id)
            || slice.session_ids.iter().any(|owner| owner != session_id)
        {
            return Err(binding_error("slice belongs to another Room"));
        }
        if slice.environment_session_id.as_deref() == Some(session_id) {
            return Ok(slice.clone());
        }
        if state.active_operations.contains_key(&slice.id)
            || state.environment_uses.contains_key(&slice.id)
        {
            return Err(binding_error(
                "slice operation in progress; retry after it completes",
            ));
        }
        let mut updated = slice.clone();
        updated.environment_session_id = Some(session_id.to_string());
        updated.updated_at_ms = now_ms;
        persist(&updated)?;
        state.records.insert(updated.id.clone(), updated.clone());
        Ok(updated)
    }
}

fn require_environment_session(
    slice: &SliceRecord,
    session_id: Option<&str>,
) -> Result<(), DaemonError> {
    if slice
        .environment_session_id
        .as_deref()
        .is_some_and(|owner| Some(owner) != session_id)
    {
        return Err(access_error("slice belongs to another Room"));
    }
    Ok(())
}

fn has_shared_worker(slice: &SliceRecord, state: &SliceStoreState) -> bool {
    state.records.values().any(|other| {
        other.id != slice.id
            && worker_refs(other).any(|left| worker_refs(slice).any(|right| left == right))
    })
}

fn access_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "environment.slice.access",
        message: format!("environment_slice_access_denied: {message}"),
    }
}

fn binding_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "environment.slice.bind",
        message: format!("environment_slice_binding_rejected: {message}"),
    }
}

fn worker_refs(slice: &SliceRecord) -> impl Iterator<Item = &str> {
    // Hosted workers share their authenticated parent Machine. Only kernel
    // identities distinguish those workers; retain synthetic legacy Machine
    // references for private/self-hosted placement compatibility.
    std::iter::once(slice.worker_kernel_ref.as_str())
        .chain(slice.worker_kernel_id.as_deref())
        .chain(
            slice
                .worker_machine_id
                .as_deref()
                .filter(|machine| *machine != slice.owner_machine_id),
        )
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

impl SliceOperationGuard {
    /// A lifecycle operation already owns the slice exclusively. Its typed
    /// guard may admit only that slice and its Room during agent relaunch.
    pub(crate) fn require_environment_use(
        &self,
        store: &SliceStore,
        slice_id: &str,
        session_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        if !Arc::ptr_eq(&self.store.inner, &store.inner)
            || self.slice_id != slice_id
            || self.operation.is_none()
        {
            return Err(access_error(
                "slice recovery requires its own active operation guard",
            ));
        }
        let state = store
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.active_operations.get(slice_id) != self.operation.as_ref() {
            return Err(access_error(
                "slice recovery operation guard is no longer active",
            ));
        }
        let slice = state
            .records
            .get(slice_id)
            .ok_or_else(|| access_error("unknown slice"))?;
        if has_shared_worker(slice, &state) {
            return Err(access_error(
                "slice worker reference is shared by another slice",
            ));
        }
        require_environment_session(slice, session_id)
    }
}

#[cfg(test)]
mod queue_tests;
