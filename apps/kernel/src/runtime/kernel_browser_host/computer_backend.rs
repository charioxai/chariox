//! MP-11 / macOS M1: native seats behind the one shared Computer adapter.
//! Linux drives the host browser process's owned Xvfb desktop; macOS registers
//! the kernel-paired Computer helper. Admission, actors, grants and epochs stay
//! in `computer.rs`; a backend only executes kernel-admitted requests.
use super::*;
use crate::error::HostFailure;

pub(super) type ComputerSeat = Arc<Mutex<dyn ComputerBackend>>;

pub(crate) trait ComputerBackend: Send {
    fn ready(&mut self) -> bool;
    fn start(&mut self) -> Result<(), String>;
    fn stop(&mut self) -> Result<(), String>;
    fn request(
        &mut self,
        method: &str,
        params: Value,
        cancellation: Option<Arc<BrowserCancellation>>,
    ) -> Result<Value, HostFailure>;
    /// Browser tabs share the Linux desktop ledger; a native-only seat has none.
    fn browser_state(&mut self) -> Result<Option<Value>, HostFailure> {
        Ok(None)
    }
    /// A kernel-owned Linux desktop may start for a focused agent. A personal
    /// seat starts only by explicit owner action, including after a crash.
    fn agent_may_start(&self) -> bool {
        false
    }
}

impl ComputerBackend for BrowserControllerProcessStdioBackend {
    fn ready(&mut self) -> bool {
        matches!(self.health(), Ok(h) if h.state == BrowserControllerProcessState::Ready)
    }
    fn start(&mut self) -> Result<(), String> {
        BrowserControllerProcessBackend::start(self).map(drop)
    }
    fn stop(&mut self) -> Result<(), String> {
        BrowserControllerProcessBackend::stop(self)
    }
    fn request(
        &mut self,
        method: &str,
        params: Value,
        cancellation: Option<Arc<BrowserCancellation>>,
    ) -> Result<Value, HostFailure> {
        self.host_request_cancellable(method, params, cancellation)
    }
    fn browser_state(&mut self) -> Result<Option<Value>, HostFailure> {
        self.host_request_classified("host.browser", serde_json::json!({"op":"state"}))
            .map(Some)
    }
    fn agent_may_start(&self) -> bool {
        true
    }
}

impl KernelBrowserHost {
    #[cfg(any(test, target_os = "macos"))]
    pub(super) fn require_computer_owner(user: &str) -> Result<(), String> {
        // Cloud owner requests have already been mapped to this canonical ID.
        // Room membership and arrival order never confer the personal Mac seat.
        if user != crate::session::DEFAULT_LOCAL_USER_ID {
            return Err("MP-11: the Computer seat requires the kernel authority owner".into());
        }
        Ok(())
    }

    /// Admit the owner before configuration and serialize first-use creation.
    #[cfg(any(test, target_os = "macos"))]
    pub(super) fn computer_seat_or_create(
        &self,
        user: &str,
        create: impl FnOnce() -> Result<Option<ComputerSeat>, String>,
    ) -> Result<Option<ComputerSeat>, String> {
        Self::require_computer_owner(user)?;
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "MP-11: desktop seat lock unavailable")?;
        if state.stopped {
            return Err("MD integration: browser host is shut down".into());
        }
        if let Some(seat) = state.seats.get(user) {
            return Ok(Some(seat.clone()));
        }
        let seat = create()?;
        if let Some(seat) = &seat {
            state.seats.insert(user.into(), seat.clone());
        }
        Ok(seat)
    }

    #[cfg(test)]
    pub(crate) fn register_computer_seat(
        &self,
        user: &str,
        seat: ComputerSeat,
    ) -> Result<(), String> {
        self.computer_seat_or_create(user, || Ok(Some(seat)))
            .map(drop)
    }

    pub(super) fn computer_backend(&self, user: &str) -> Result<ComputerSeat, String> {
        #[cfg(target_os = "macos")]
        let seat = self.computer_seat_or_create(user, || {
            super::mac_helper::MacComputerHelper::configured(&self.root)
                .map(|helper| helper.map(|helper| Arc::new(Mutex::new(helper)) as ComputerSeat))
        })?;
        #[cfg(not(target_os = "macos"))]
        let seat = self
            .inner
            .lock()
            .map_err(|_| "MP-11: desktop seat lock unavailable")?
            .seats
            .get(user)
            .cloned();
        if let Some(seat) = seat {
            return Ok(seat);
        }
        Ok(self.backend(user)?)
    }

    /// Stop completes before Revoke all returns, so the next start is fresh.
    /// Backends may reap an already fenced helper asynchronously.
    pub(super) fn stop_seat(seat: Option<ComputerSeat>) {
        if let Some(seat) = seat {
            let _ = seat.lock().unwrap_or_else(|e| e.into_inner()).stop();
        }
    }
}
