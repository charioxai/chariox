//! Claim a capture before any external command or file read can run.
use super::*;

#[derive(Clone, Default)]
pub(in crate::runtime::state) struct CaptureClaims(Arc<std::sync::Mutex<BTreeSet<String>>>);

pub(super) struct CaptureClaim {
    claims: CaptureClaims,
    handle: String,
}

impl CaptureClaims {
    pub(super) fn claim(&self, handle: &str) -> Result<Arc<CaptureClaim>, DaemonError> {
        if !self
            .0
            .lock()
            .map_err(|_| error("Vault capture claims are unavailable"))?
            .insert(handle.into())
        {
            return Err(error("this Vault capture request is already running; wait for its result before retrying"));
        }
        Ok(Arc::new(CaptureClaim {
            claims: self.clone(),
            handle: handle.into(),
        }))
    }
}

impl Drop for CaptureClaim {
    fn drop(&mut self) {
        if let Ok(mut claims) = self.claims.0.lock() {
            claims.remove(&self.handle);
        }
    }
}
