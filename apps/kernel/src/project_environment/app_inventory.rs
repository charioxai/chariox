//! Bounded App inventory for Environment admission and live binding checks.
use super::environment_error;
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use chariox_app_runtime::installation::Installation;

pub(crate) fn environment_app_installations(
    store: &DurableKernelStateStore,
    owner: &str,
) -> Result<Vec<Installation>, DaemonError> {
    let mut installations = Vec::new();
    let mut after = None;
    // The registry admits 100 entries per page; Environment review bounds the
    // complete inventory rather than silently ignoring later installations.
    for _ in 0..10 {
        let page = store
            .list_app_installations(owner, after.as_deref(), 100)
            .map_err(|_| environment_error("App registry unavailable"))?;
        installations.extend(page.installations);
        match page.next_cursor {
            Some(cursor) => after = Some(cursor),
            None => return Ok(installations),
        }
    }
    Err(environment_error(
        "App inventory exceeds the 1,000 installation review capacity; reduce the installed set and retry",
    ))
}
