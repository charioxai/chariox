//! The requests a kernel sends to a source kernel for a notification subscription. They run
//! outside the command that started them, so each carries the generation it started under and
//! the store applies its answer only while that generation is still current. An answer that
//! arrives late is dropped, and the source is asked for the current state again.
use super::workflow_notification_peers::VERSION;
use super::*;
use crate::durable_state::workflow_notifications::{
    remote_install::{Generation, RemoteReconcile},
    NotificationOperation, NotificationOutcome,
};
use crate::local::WorkflowNotificationSubscription;

/// The generation as the source kernel receives it. Generations start at one.
fn wire(generation: Generation) -> u64 {
    u64::try_from(generation).unwrap_or_default()
}

impl KernelRuntimeState {
    /// Detaches here, then on the source kernel when it is another one.
    pub(super) async fn detach_notification_subscription(
        &self,
        subscription: &WorkflowNotificationSubscription,
    ) -> Result<(), DaemonError> {
        let NotificationOutcome::Detached { generation } =
            self.owned
                .durable_state_store
                .notify(NotificationOperation::Detach {
                    subscription_id: subscription.subscription_id.clone(),
                    owner: subscription.owner_user_id.clone(),
                    kernel: subscription.target_kernel_id.clone(),
                })?
        else {
            return Err(crate::durable_state::workflow_notifications::error(
                "unexpected notification response",
            ));
        };
        if let (Some(generation), true) = (
            generation,
            subscription.source_kernel_id != subscription.target_kernel_id,
        ) {
            self.drop_from_source(subscription, generation).await?;
        }
        Ok(())
    }

    /// Asks the source kernel to drop a subscription this kernel detached under `generation`.
    pub(super) async fn drop_from_source(
        &self,
        subscription: &WorkflowNotificationSubscription,
        generation: Generation,
    ) -> Result<(), DaemonError> {
        let owner = &subscription.owner_user_id;
        let id = &subscription.subscription_id;
        if !self
            .owned
            .durable_state_store
            .remote_work_is_current(owner, id, generation)?
        {
            return Ok(());
        }
        let response = self
            .notification_peer(
                owner,
                &subscription.source_kernel_id,
                RelayPeerRequest::UnsubscribeWorkflowNotifications {
                    protocol_version: VERSION,
                    subscription_id: id.clone(),
                    generation: wire(generation),
                },
            )
            .await;
        self.owned
            .durable_state_store
            .notify(NotificationOperation::RemoteDetachSettled {
                owner: owner.clone(),
                subscription_id: id.clone(),
                source_kernel: subscription.source_kernel_id.clone(),
                generation,
                confirmed: matches!(
                    response,
                    Ok(RelayPeerResponse::WorkflowNotificationUnsubscribed)
                ),
            })?;
        Ok(())
    }

    /// Installs a committed attach on its source kernel. Answers whether the source holds it;
    /// a refused or unanswered install has put the replaced subscription back.
    pub(super) async fn install_on_source(
        &self,
        subscription: &WorkflowNotificationSubscription,
        generation: Generation,
    ) -> Result<bool, DaemonError> {
        let owner = &subscription.owner_user_id;
        let id = &subscription.subscription_id;
        if !self
            .owned
            .durable_state_store
            .remote_work_is_current(owner, id, generation)?
        {
            // A newer attach or detach owns the subscription and installs or drops it itself.
            return Ok(true);
        }
        let response = self
            .notification_peer(
                owner,
                &subscription.source_kernel_id,
                RelayPeerRequest::SubscribeWorkflowNotifications {
                    protocol_version: VERSION,
                    source_id: subscription.source_id.clone(),
                    target_ref: subscription.clone(),
                    generation: wire(generation),
                },
            )
            .await;
        let confirmed = matches!(&response, Ok(RelayPeerResponse::WorkflowNotificationSubscribed { subscription: s }) if s == subscription);
        let (owner, subscription_id) = (owner.clone(), id.clone());
        self.owned.durable_state_store.notify(if confirmed {
            NotificationOperation::RemoteInstallConfirmed {
                owner,
                subscription_id,
                generation,
            }
        } else {
            // The source may or may not have installed it: the row goes back to what it was
            // and the source is reconciled, never assumed empty.
            NotificationOperation::RemoteInstallFailed {
                owner,
                subscription_id,
                generation,
            }
        })?;
        Ok(confirmed)
    }

    /// Brings a source kernel in line with the state an unconfirmed attach or detach left
    /// here: it re-installs the subscription this kernel kept, or drops the one it should not
    /// hold. Each source is asked once per retry window, with the request timeout.
    pub(super) async fn reconcile_remote_notification_installs(&self, now: u64) {
        let connected = self
            .owned
            .notification_inventory_projection
            .take_appeared_kernels();
        if !connected.is_empty() {
            let _ = self
                .owned
                .durable_state_store
                .notify(NotificationOperation::RemoteKernelsConnected { kernels: connected });
        }
        let _ = self
            .owned
            .durable_state_store
            .notify(NotificationOperation::SettleInterruptedRemoteInstalls { now });
        let Ok(due) = self.claim_remote_reconciles(now) else {
            return;
        };
        let mut asks = tokio::task::JoinSet::new();
        for item in due {
            let runtime = self.clone();
            asks.spawn(async move { runtime.ask_remote_reconcile(item).await });
        }
        while asks.join_next().await.is_some() {}
    }

    pub(super) fn claim_remote_reconciles(
        &self,
        now: u64,
    ) -> Result<Vec<RemoteReconcile>, DaemonError> {
        match self
            .owned
            .durable_state_store
            .notify(NotificationOperation::ClaimRemoteReconciles { now, limit: 8 })?
        {
            NotificationOutcome::Reconciles(due) => Ok(due),
            _ => Ok(Vec::new()),
        }
    }

    /// Sends one claimed reconciliation, unless a newer attach or detach has replaced it since.
    pub(super) async fn ask_remote_reconcile(&self, item: RemoteReconcile) {
        let current = self.owned.durable_state_store.remote_work_is_current(
            &item.owner,
            &item.subscription_id,
            item.generation,
        );
        if !current.unwrap_or(false) {
            return;
        }
        let request = match &item.desired {
            Some(subscription) => RelayPeerRequest::SubscribeWorkflowNotifications {
                protocol_version: VERSION,
                source_id: subscription.source_id.clone(),
                target_ref: subscription.clone(),
                generation: wire(item.generation),
            },
            None => RelayPeerRequest::UnsubscribeWorkflowNotifications {
                protocol_version: VERSION,
                subscription_id: item.subscription_id.clone(),
                generation: wire(item.generation),
            },
        };
        let response = self
            .notification_peer(&item.owner, &item.source_kernel_id, request)
            .await;
        let matched = match (&item.desired, response) {
            (
                Some(want),
                Ok(RelayPeerResponse::WorkflowNotificationSubscribed { subscription }),
            ) => subscription == *want,
            (None, Ok(RelayPeerResponse::WorkflowNotificationUnsubscribed)) => true,
            _ => false,
        };
        let _ =
            self.owned
                .durable_state_store
                .notify(NotificationOperation::RemoteReconcileSettled {
                    owner: item.owner,
                    subscription_id: item.subscription_id,
                    generation: item.generation,
                    matched,
                });
    }
}
