//! A connected terminal's hold on its browser actor, whichever transport carries it.
use super::KernelBrowserHost;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Live holds per `(user, actor)`. The actor is disconnected when the last one is dropped.
pub(super) type TerminalLeases = Arc<Mutex<BTreeMap<(String, String), usize>>>;

/// Keeps `actor` registered for `user` while the connection that uses it is alive. Dropping the
/// last hold, whether the peer said goodbye or vanished, disconnects the actor exactly once.
pub(crate) struct TerminalLease {
    host: KernelBrowserHost,
    context: (String, String),
}

impl std::fmt::Debug for TerminalLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TerminalLease")
    }
}

impl KernelBrowserHost {
    pub(crate) fn lease_terminal(&self, user: &str, actor: &str) -> TerminalLease {
        let context = (user.to_string(), actor.to_string());
        *self
            .terminal_leases
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(context.clone())
            .or_default() += 1;
        TerminalLease {
            host: self.clone(),
            context,
        }
    }
}

impl Drop for TerminalLease {
    fn drop(&mut self) {
        // Held across the disconnect so a new hold on the same actor starts after it.
        let mut leases = self
            .host
            .terminal_leases
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(count) = leases.get_mut(&self.context) else {
            return;
        };
        *count -= 1;
        if *count == 0 {
            leases.remove(&self.context);
            self.host
                .disconnect_terminal(&self.context.0, &self.context.1);
        }
    }
}
