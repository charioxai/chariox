use crate::config::{DaemonConfig, EventGeneratorManagementTarget};
use crate::error::DaemonError;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

#[derive(Clone)]
pub(crate) struct DaemonConfigProjectionStore {
    config: Arc<StdMutex<DaemonConfig>>,
    /// A renewed token the durable save has not caught up with. App configuration published
    /// meanwhile still carries the previous token: it must not take the renewal back.
    renewal: Arc<StdMutex<Option<PendingRenewal>>>,
    observers: Arc<StdMutex<Vec<ConfigObserver>>>,
}

struct PendingRenewal {
    token: String,
    profile: crate::config::PersistedCloudRelayProfile,
    /// The transport and removal state the renewal was installed over. Only a config that still
    /// carries exactly these is a stale publish of the same state; anything else is a decision
    /// (a private relay, a recorded owner removal, a saved token) that the renewal must not undo.
    base_relay_url: Option<String>,
    base_relay_token: Option<String>,
    base_removal_pending: bool,
    /// Earlier renewals of this chain whose saves have not landed, oldest first. A save of one
    /// of them moves the baseline forward; it does not end the newer renewal.
    unsaved_earlier: Vec<String>,
}

/// How a renewal's delayed save stands against the projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenewalSave {
    /// No unsaved renewal is tracked: the save is judged on the app's config alone.
    Untracked,
    /// The renewal is still in the unsaved chain; the app and the disk carry this removal mark.
    Unsaved { base_removal_pending: bool },
    /// A newer renewal was saved, or a decision (private relay, removal, new enrollment) ended
    /// the chain: this save must not land.
    Superseded,
}

/// The same Cloud enrollment: a renewal changes only the token's expiry and the removal-pending mark.
pub(crate) fn same_cloud_enrollment(
    left: &crate::config::PersistedCloudRelayProfile,
    right: &crate::config::PersistedCloudRelayProfile,
) -> bool {
    let normalized = |profile: &crate::config::PersistedCloudRelayProfile| {
        let mut profile = profile.clone();
        profile.token_expires_at_ms = None;
        profile.kernel_removal_pending = false;
        profile
    };
    normalized(left) == normalized(right)
}

/// Called on the updating thread, after the new config is visible and no lock is held.
type ConfigObserver = Arc<dyn Fn(&DaemonConfig) + Send + Sync>;

impl DaemonConfigProjectionStore {
    pub(crate) fn new(config: DaemonConfig) -> Self {
        Self {
            config: Arc::new(StdMutex::new(config)),
            renewal: Arc::new(StdMutex::new(None)),
            observers: Arc::new(StdMutex::new(Vec::new())),
        }
    }

    pub(crate) fn snapshot(&self) -> DaemonConfig {
        self.config
            .lock()
            .expect("daemon config projection lock should not be poisoned")
            .clone()
    }

    pub(crate) fn set_runtime_mcp_port(&self, port: u16) {
        self.config
            .lock()
            .expect("daemon config projection lock should not be poisoned")
            .runtime_mcp_port = port;
    }

    pub(crate) fn local_owner_user_id(&self) -> String {
        self.config
            .lock()
            .expect("daemon config projection lock should not be poisoned")
            .cloud_relay
            .as_ref()
            .map(|profile| profile.user_id.clone())
            .unwrap_or_else(|| crate::session::DEFAULT_LOCAL_USER_ID.into())
    }

    /// Pre-enrollment rooms keep their member identity after the owner enrolls.
    pub(crate) fn room_member_user_id_in(&self, room_owner: &str, user_id: &str) -> String {
        room_member_user_id_in(
            &self.config.lock().expect("daemon config projection lock"),
            room_owner,
            user_id,
        )
    }

    pub(crate) fn workflow_invoker_matches(
        &self,
        kernel_id: &str,
        room_owner: &str,
        user_id: &str,
        invoker: &str,
    ) -> bool {
        let config = self.config.lock().expect("daemon config projection lock");
        config.daemon_id == kernel_id
            && room_member_user_id_in(&config, room_owner, user_id)
                == room_member_user_id_in(&config, room_owner, invoker)
    }

    /// The enrolled owner, or the `local` identity this owner's pre-enrollment rooms keep: the
    /// terminal reaches those under `local` for membership.
    pub(crate) fn is_kernel_owner(&self, kernel_id: &str, user_id: &str) -> bool {
        let config = self.config.lock().expect("daemon config projection lock");
        config.daemon_id == kernel_id
            && (user_id == crate::session::DEFAULT_LOCAL_USER_ID
                || config
                    .cloud_relay
                    .as_ref()
                    .is_some_and(|profile| profile.user_id == user_id))
    }

    /// Installs a renewed Cloud relay token where the connector and every other relay sender read
    /// it, without the app lock: a durable write stalled on a slow disk must not hold back
    /// reauthentication. The durable write follows; `previous` is the snapshot the renewal was
    /// requested for, so a renewal for a replaced enrollment is refused.
    pub(crate) fn install_cloud_relay_renewal(
        &self,
        previous: &DaemonConfig,
        profile: crate::config::PersistedCloudRelayProfile,
        token: String,
    ) -> Result<(), DaemonError> {
        let installed = {
            let mut config = self
                .config
                .lock()
                .expect("daemon config projection lock should not be poisoned");
            let unchanged = config.relay_url == previous.relay_url
                && config.relay_token == previous.relay_token
                && config.relay_public_key == previous.relay_public_key
                && config.host_machine_id == previous.host_machine_id
                && config.cloud_relay == previous.cloud_relay;
            if !unchanged {
                return Err(DaemonError::LocalTransport {
                    operation: "install renewed relay token",
                    message: "relay configuration changed during renewal; retry with the current configuration".into(),
                });
            }
            config.relay_url = Some(profile.relay_url.clone());
            config.relay_token = Some(token.clone());
            config.cloud_relay = Some(profile.clone());
            let mut pending = self
                .renewal
                .lock()
                .expect("daemon config renewal lock should not be poisoned");
            // Renewals chained over unsaved ones keep the first baseline: it is what the app and
            // the disk still carry until a save lands.
            let (base_relay_url, base_relay_token, base_removal_pending, unsaved_earlier) =
                match pending.take() {
                    Some(existing) => {
                        let mut unsaved_earlier = existing.unsaved_earlier;
                        unsaved_earlier.push(existing.token);
                        (
                            existing.base_relay_url,
                            existing.base_relay_token,
                            existing.base_removal_pending,
                            unsaved_earlier,
                        )
                    }
                    None => (
                        previous.relay_url.clone(),
                        previous.relay_token.clone(),
                        previous
                            .cloud_relay
                            .as_ref()
                            .is_some_and(|profile| profile.kernel_removal_pending),
                        Vec::new(),
                    ),
                };
            *pending = Some(PendingRenewal {
                token,
                profile,
                base_relay_url,
                base_relay_token,
                base_removal_pending,
                unsaved_earlier,
            });
            config.clone()
        };
        self.notify_observers(&installed);
        Ok(())
    }

    /// The removal mark the app and the disk still carry while renewals are unsaved: a save is
    /// accepted against that, not against the projected profile that already shows the renewal.
    pub(crate) fn unsaved_renewal_removal_baseline(&self) -> Option<bool> {
        self.renewal
            .lock()
            .expect("daemon config renewal lock should not be poisoned")
            .as_ref()
            .map(|renewal| renewal.base_removal_pending)
    }

    /// Asked under the app lock by a renewal's delayed save, before it writes `token`.
    pub(crate) fn renewal_save(&self, token: &str) -> RenewalSave {
        match self
            .renewal
            .lock()
            .expect("daemon config renewal lock should not be poisoned")
            .as_ref()
        {
            None => RenewalSave::Untracked,
            Some(renewal)
                if renewal.token == token
                    || renewal
                        .unsaved_earlier
                        .iter()
                        .any(|earlier| earlier == token) =>
            {
                RenewalSave::Unsaved {
                    base_removal_pending: renewal.base_removal_pending,
                }
            }
            Some(_) => RenewalSave::Superseded,
        }
    }

    fn notify_observers(&self, config: &DaemonConfig) {
        let observers = self
            .observers
            .lock()
            .expect("daemon config observers lock should not be poisoned")
            .clone();
        for observer in observers {
            observer(config);
        }
    }

    pub(crate) fn update(&self, mut config: DaemonConfig) {
        {
            let mut projected = self
                .config
                .lock()
                .expect("daemon config projection lock should not be poisoned");
            let mut pending = self
                .renewal
                .lock()
                .expect("daemon config renewal lock should not be poisoned");
            if let Some(renewal) = pending.as_mut() {
                let saved_earlier = config
                    .relay_token
                    .as_ref()
                    .and_then(|token| {
                        renewal
                            .unsaved_earlier
                            .iter()
                            .position(|earlier| earlier == token)
                    })
                    .filter(|_| {
                        config.relay_url.as_deref() == Some(renewal.profile.relay_url.as_str())
                            && config.cloud_relay.as_ref().is_some_and(|profile| {
                                same_cloud_enrollment(profile, &renewal.profile)
                            })
                    });
                if let Some(position) = saved_earlier {
                    // An earlier renewal of the chain was saved: that is the new baseline.
                    renewal.base_relay_url = config.relay_url.clone();
                    renewal.base_relay_token = config.relay_token.clone();
                    renewal.base_removal_pending = config
                        .cloud_relay
                        .as_ref()
                        .is_some_and(|profile| profile.kernel_removal_pending);
                    renewal.unsaved_earlier.drain(..=position);
                }
                let stale_publish = config.relay_url == renewal.base_relay_url
                    && config.relay_token == renewal.base_relay_token
                    && config.cloud_relay.as_ref().is_some_and(|profile| {
                        same_cloud_enrollment(profile, &renewal.profile)
                            && profile.kernel_removal_pending == renewal.base_removal_pending
                    });
                if stale_publish {
                    config.relay_url = Some(renewal.profile.relay_url.clone());
                    config.relay_token = Some(renewal.token.clone());
                    config.cloud_relay = Some(renewal.profile.clone());
                } else {
                    *pending = None;
                }
            }
            *projected = config.clone();
        }
        self.notify_observers(&config);
    }

    /// Runs `observer` inside every later `update`, before the caller of `update` goes on. A revocation that must take
    /// effect before its own reply (a logout) cannot wait for a poll of the snapshot.
    pub(crate) fn observe_updates(&self, observer: impl Fn(&DaemonConfig) + Send + Sync + 'static) {
        self.observers
            .lock()
            .expect("daemon config observers lock should not be poisoned")
            .push(Arc::new(observer));
    }

    /// Remember the management URL the registry gave for a generator, keeping any target the
    /// operator configured.
    pub(crate) fn merge_event_generator_management_target(
        &self,
        generator_id: &str,
        resolved: EventGeneratorManagementTarget,
    ) {
        self.config
            .lock()
            .expect("daemon config projection lock should not be poisoned")
            .event_generator_management_targets
            .entry(generator_id.to_string())
            .or_insert(resolved);
    }
}

fn room_member_user_id_in(config: &DaemonConfig, room_owner: &str, user_id: &str) -> String {
    if room_owner == crate::session::DEFAULT_LOCAL_USER_ID
        && !user_id.is_empty()
        && config
            .cloud_relay
            .as_ref()
            .is_some_and(|profile| profile.user_id == user_id)
    {
        crate::session::DEFAULT_LOCAL_USER_ID.into()
    } else {
        user_id.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enrolled() -> DaemonConfig {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".into());
        config.relay_token = Some("first".into());
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            relay_url: "wss://relay.example.test".into(),
            kernel_id: Some(config.daemon_id.clone()),
            kernel_credential: Some("enrollment".into()),
            token_expires_at_ms: Some(1_000),
            ..Default::default()
        });
        config
    }

    fn renewed_profile(config: &DaemonConfig) -> crate::config::PersistedCloudRelayProfile {
        let mut profile = config.cloud_relay.clone().unwrap();
        profile.token_expires_at_ms = Some(9_000);
        profile
    }

    /// Row 563 review: an app configuration write that stalled on the disk publishes the config it
    /// read before the renewal. It must not take the renewed token back out of the projection.
    #[test]
    fn unrelated_app_config_publish_keeps_an_unsaved_renewal() {
        let config = enrolled();
        let projection = DaemonConfigProjectionStore::new(config.clone());
        projection
            .install_cloud_relay_renewal(&config, renewed_profile(&config), "renewed".into())
            .unwrap();
        let mut stale = config.clone();
        stale.runtime_mcp_port = 4321;
        projection.update(stale);
        let seen = projection.snapshot();
        assert_eq!(seen.relay_token.as_deref(), Some("renewed"));
        assert_eq!(seen.cloud_relay.unwrap().token_expires_at_ms, Some(9_000));
        assert_eq!(seen.runtime_mcp_port, 4321);
    }

    /// Row 563 review: a private relay chosen, or an owner removal recorded, while a renewal is
    /// unsaved are decisions, not stale publishes: the renewal must not take them back.
    #[test]
    fn a_private_relay_or_a_recorded_removal_ends_the_overlay() {
        let config = enrolled();
        let projection = DaemonConfigProjectionStore::new(config.clone());
        projection
            .install_cloud_relay_renewal(&config, renewed_profile(&config), "renewed".into())
            .unwrap();
        let mut private = config.clone();
        private.relay_url = Some("ws://127.0.0.1:9010".into());
        private.relay_token = Some("private".into());
        projection.update(private);
        let seen = projection.snapshot();
        assert_eq!(seen.relay_url.as_deref(), Some("ws://127.0.0.1:9010"));
        assert_eq!(seen.relay_token.as_deref(), Some("private"));

        let projection = DaemonConfigProjectionStore::new(config.clone());
        projection
            .install_cloud_relay_renewal(&config, renewed_profile(&config), "renewed".into())
            .unwrap();
        let mut removed = config.clone();
        removed.relay_token = None;
        removed.cloud_relay.as_mut().unwrap().kernel_removal_pending = true;
        removed.cloud_relay.as_mut().unwrap().token_expires_at_ms = None;
        projection.update(removed);
        let seen = projection.snapshot();
        assert_eq!(seen.relay_token, None);
        assert!(seen.cloud_relay.unwrap().kernel_removal_pending);
    }

    /// Row 563 review: a second renewal installed over an unsaved first one must not make the
    /// first renewal's baseline the new "saved" config.
    #[test]
    fn chained_unsaved_renewals_keep_the_durable_baseline() {
        let mut config = enrolled();
        config.cloud_relay.as_mut().unwrap().kernel_removal_pending = true;
        config.relay_token = None;
        let projection = DaemonConfigProjectionStore::new(config.clone());
        let mut first = renewed_profile(&config);
        first.kernel_removal_pending = false;
        projection
            .install_cloud_relay_renewal(&config, first, "t1".into())
            .unwrap();
        let mut second = renewed_profile(&config);
        second.kernel_removal_pending = false;
        second.token_expires_at_ms = Some(19_000);
        projection
            .install_cloud_relay_renewal(&projection.snapshot(), second, "t2".into())
            .unwrap();
        assert_eq!(projection.unsaved_renewal_removal_baseline(), Some(true));
        let mut unrelated = config.clone();
        unrelated.runtime_mcp_port = 4321;
        projection.update(unrelated);
        let seen = projection.snapshot();
        assert_eq!(seen.relay_token.as_deref(), Some("t2"));
        assert_eq!(seen.runtime_mcp_port, 4321);
    }

    #[test]
    fn the_saved_renewal_or_a_replaced_enrollment_ends_the_overlay() {
        let config = enrolled();
        let projection = DaemonConfigProjectionStore::new(config.clone());
        projection
            .install_cloud_relay_renewal(&config, renewed_profile(&config), "renewed".into())
            .unwrap();
        let mut saved = config.clone();
        saved.relay_token = Some("renewed".into());
        saved.cloud_relay = Some(renewed_profile(&config));
        projection.update(saved.clone());
        let mut later = config.clone();
        later.relay_token = Some("first".into());
        projection.update(later);
        assert_eq!(projection.snapshot().relay_token.as_deref(), Some("first"));

        projection
            .install_cloud_relay_renewal(
                &projection.snapshot(),
                renewed_profile(&config),
                "again".into(),
            )
            .unwrap();
        let mut relinked = config.clone();
        relinked.cloud_relay.as_mut().unwrap().kernel_credential = Some("other".into());
        projection.update(relinked);
        assert_eq!(projection.snapshot().relay_token.as_deref(), Some("first"));
    }
}
