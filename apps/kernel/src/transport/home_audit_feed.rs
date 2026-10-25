//! MP-08 / MP-10 / MP-11: the home extension audit rows appended while one session subscription
//! lives. A client adds each row to the list it read once instead of reading the list again.
use std::collections::{BTreeSet, VecDeque};

use tokio::sync::broadcast::error::RecvError;

use crate::durable_state::DurableStateEvent;
use crate::runtime::router::CommandRouter;
use crate::transport::kernel_protocol::KernelEvent;

pub(crate) struct HomeAuditFeed {
    appended: tokio::sync::broadcast::Receiver<DurableStateEvent>,
    pending: VecDeque<DurableStateEvent>,
}

impl HomeAuditFeed {
    /// Subscribe before the client's first read, so no row falls between the read and the feed.
    pub(crate) fn new(router: &CommandRouter) -> Self {
        Self {
            appended: router.subscribe_durable_state_events(),
            pending: VecDeque::new(),
        }
    }

    /// Resolves once an appended row waits to be sent; never resolves when the feed has ended.
    pub(crate) async fn changed(&mut self) {
        if !self.pending.is_empty() {
            return;
        }
        loop {
            match self.appended.recv().await {
                Ok(event) => {
                    self.pending.push_back(event);
                    return;
                }
                Err(RecvError::Lagged(missed)) => {
                    crate::logging::warn_with_fields(
                        "daemon.runtime_transport",
                        "a subscription missed home extension audit rows; the client's audit list is stale until it reads it again",
                        serde_json::json!({ "missed": missed }),
                    );
                }
                Err(RecvError::Closed) => std::future::pending::<()>().await,
            }
        }
    }

    /// The rows of the session's agents that the attachment's owner may read.
    pub(crate) async fn next_events(
        &mut self,
        router: &CommandRouter,
        session_id: &str,
        attachment_id: &str,
        session_agents: Option<&BTreeSet<String>>,
    ) -> Vec<KernelEvent> {
        loop {
            match self.appended.try_recv() {
                Ok(event) => self.pending.push_back(event),
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(missed)) => {
                    crate::logging::warn_with_fields(
                        "daemon.runtime_transport",
                        "a subscription missed home extension audit rows; the client's audit list is stale until it reads it again",
                        serde_json::json!({ "missed": missed }),
                    );
                }
                Err(_) => break,
            }
        }
        let Some(session_agents) = session_agents else {
            // Before the first snapshot the session's agents are unknown; the view's reads cover it.
            self.pending.clear();
            return Vec::new();
        };
        if self.pending.is_empty() {
            return Vec::new();
        }
        let Ok(owner_user_id) = router
            .runtime_state()
            .attachment_owner_user_id(attachment_id)
            .await
        else {
            self.pending.clear();
            return Vec::new();
        };
        std::mem::take(&mut self.pending)
            .into_iter()
            .filter_map(|event| {
                let agent_id = event.subject_id.clone()?;
                (session_agents.contains(&agent_id)
                    && router
                        .runtime_state()
                        .is_readable_home_extension_audit_event(&event, &owner_user_id))
                .then(|| KernelEvent::HomeExtensionAuditAppended {
                    session_id: session_id.to_string(),
                    agent_id,
                    audit_event: Box::new(event),
                })
            })
            .collect()
    }
}
