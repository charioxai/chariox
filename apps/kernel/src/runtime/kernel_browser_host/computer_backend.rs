//! MP-11 / macOS M1: native seats behind the one shared Computer adapter.
//! Linux drives the host browser process's owned Xvfb desktop; macOS registers
//! the kernel-paired Computer helper. Admission, actors, grants and epochs stay
//! in `computer.rs`; a backend only executes kernel-admitted requests.
use super::*;
use crate::error::HostFailure;

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
    /// Registers a native seat for `user`. The host owns at most one personal
    /// seat: another user cannot adopt or take it from its live owner.
    pub(crate) fn register_computer_seat(
        &self,
        user: &str,
        seat: Arc<Mutex<dyn ComputerBackend>>,
    ) -> Result<(), String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "MP-11: desktop seat lock unavailable")?;
        if state.stopped {
            return Err("MD integration: browser host is shut down".into());
        }
        if state.seats.keys().any(|owner| owner != user) {
            return Err("MP-11: the Computer seat belongs to another user".into());
        }
        state.seats.insert(user.into(), seat);
        Ok(())
    }

    pub(super) fn computer_backend(
        &self,
        user: &str,
    ) -> Result<Arc<Mutex<dyn ComputerBackend>>, String> {
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
        #[cfg(target_os = "macos")]
        if let Some(helper) = super::mac_helper::MacComputerHelper::configured(&self.root)? {
            let seat: Arc<Mutex<dyn ComputerBackend>> = Arc::new(Mutex::new(helper));
            self.register_computer_seat(user, seat.clone())?;
            return Ok(seat);
        }
        Ok(self.backend(user)?)
    }

    /// Explicit Stop for a personal seat: fences the helper and ends its epoch.
    /// The Linux desktop keeps its own browser-host lifecycle.
    pub(super) fn stop_seat(seat: Option<Arc<Mutex<dyn ComputerBackend>>>) {
        if let Some(seat) = seat {
            std::thread::spawn(move || {
                let _ = seat.lock().unwrap_or_else(|e| e.into_inner()).stop();
            });
        }
    }
}
