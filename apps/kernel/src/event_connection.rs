use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::local::{EventConnection, EventConnectionAuthorization};

const CONNECTION_UPSERTED: &str = "event_connection.upserted";
const CONNECTION_REMOVED: &str = "event_connection.removed";
const AUTHORIZATION_UPSERTED: &str = "event_connection.authorization.upserted";
const AUTHORIZATION_REMOVED: &str = "event_connection.authorization.removed";

#[derive(Clone)]
pub(crate) struct EventConnectionRegistry {
    durable_state: DurableKernelStateStore,
    state: Arc<Mutex<RegistryState>>,
    restore_error: Arc<Mutex<Option<String>>>,
}

#[derive(Default)]
struct RegistryState {
    connections: BTreeMap<(String, String), StoredConnection>,
    authorizations: BTreeMap<(String, String), StoredAuthorization>,
}

#[derive(Clone, Serialize, Deserialize)]
struct StoredConnection {
    owner_user_id: String,
    connection: EventConnection,
}

#[derive(Clone, Serialize, Deserialize)]
struct StoredAuthorization {
    owner_user_id: String,
    authorization: EventConnectionAuthorization,
}

#[derive(Serialize, Deserialize)]
struct RemovedRecord {
    owner_user_id: String,
    id: String,
}

// The registry uses the kernel-wide `DaemonError` contract at its runtime boundary. Boxing only
// these results would make every caller unwrap a one-off error representation.
#[allow(clippy::result_large_err)]
impl EventConnectionRegistry {
    pub(crate) fn new(durable_state: DurableKernelStateStore) -> Self {
        let registry = Self {
            durable_state,
            state: Arc::new(Mutex::new(RegistryState::default())),
            restore_error: Arc::new(Mutex::new(None)),
        };
        match registry.restore() {
            Ok(restored) => {
                *registry
                    .state
                    .lock()
                    .expect("event connection registry lock") = restored;
            }
            Err(error) => {
                *registry
                    .restore_error
                    .lock()
                    .expect("event connection restore error lock") = Some(error.to_string());
            }
        }
        registry
    }

    pub(crate) fn list(
        &self,
        owner_user_id: &str,
        generator_id: Option<&str>,
    ) -> Result<Vec<EventConnection>, DaemonError> {
        let state = self.lock_state()?;
        let mut connections = state
            .connections
            .values()
            .filter(|record| record.owner_user_id == owner_user_id)
            .map(|record| record.connection.clone())
            .filter(|connection| {
                generator_id.is_none_or(|generator_id| connection.generator_id == generator_id)
            })
            .collect::<Vec<_>>();
        connections.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.connection_id.cmp(&right.connection_id))
        });
        Ok(connections)
    }

    pub(crate) fn get(
        &self,
        owner_user_id: &str,
        connection_id: &str,
    ) -> Result<Option<EventConnection>, DaemonError> {
        Ok(self
            .lock_state()?
            .connections
            .get(&(owner_user_id.to_string(), connection_id.to_string()))
            .map(|record| record.connection.clone()))
    }

    pub(crate) fn upsert(
        &self,
        owner_user_id: &str,
        summary: chariox_event_protocol::AegsConnectionSummary,
    ) -> Result<EventConnection, DaemonError> {
        let now_ms = unix_epoch_ms();
        let key = (owner_user_id.to_string(), summary.connection_id.clone());
        let mut state = self.lock_state()?;
        let created_at_ms = state
            .connections
            .get(&key)
            .map(|record| record.connection.created_at_ms)
            .unwrap_or(now_ms);
        let previous = state.connections.get(&key).map(|record| &record.connection);
        let lifecycle_state = lifecycle_state_for_status(summary.status);
        let record = StoredConnection {
            owner_user_id: owner_user_id.to_string(),
            connection: EventConnection {
                generator_id: summary.generator_id,
                connection_id: summary.connection_id,
                status: summary.status,
                lifecycle_state,
                scopes: previous
                    .map(|value| value.scopes.clone())
                    .unwrap_or_default(),
                resources: previous
                    .map(|value| value.resources.clone())
                    .unwrap_or_default(),
                attached_trigger_count: previous
                    .map(|value| value.attached_trigger_count)
                    .unwrap_or_default(),
                metadata: summary.metadata,
                expires_at_ms: summary.expires_at_ms,
                created_at_ms,
                updated_at_ms: summary.updated_at_ms,
                last_validated_at_ms: Some(now_ms),
                last_successful_health_check_at_ms: previous
                    .and_then(|value| value.last_successful_health_check_at_ms),
                last_accepted_event_at_ms: previous
                    .and_then(|value| value.last_accepted_event_at_ms),
                problem_code: None,
                problem_message: None,
                recovery_action: None,
                test_event_supported: previous.is_some_and(|value| value.test_event_supported),
            },
        };
        self.append(
            CONNECTION_UPSERTED,
            &record.connection.connection_id,
            &record,
        )?;
        let connection = record.connection.clone();
        state.connections.insert(key, record);
        Ok(connection)
    }

    pub(crate) fn apply_inspection(
        &self,
        owner_user_id: &str,
        inspection: chariox_event_protocol::AegsConnectionInspection,
    ) -> Result<EventConnection, DaemonError> {
        let mut state = self.lock_state()?;
        let key = (owner_user_id.to_string(), inspection.connection_id.clone());
        let mut record = state
            .connections
            .get(&key)
            .cloned()
            .ok_or_else(|| registry_error("connection was not found".to_string()))?;
        if record.connection.generator_id != inspection.generator_id {
            return Err(registry_error(
                "connection inspection generator does not match the installed connection"
                    .to_string(),
            ));
        }
        record.connection.lifecycle_state = inspection.lifecycle_state;
        record.connection.status = status_for_lifecycle(inspection.lifecycle_state);
        record.connection.scopes = inspection.scopes;
        record.connection.resources = inspection.resources;
        record.connection.last_validated_at_ms = Some(unix_epoch_ms());
        record.connection.last_successful_health_check_at_ms =
            inspection.last_successful_health_check_at_ms;
        record.connection.last_accepted_event_at_ms = inspection.last_accepted_event_at_ms;
        record.connection.problem_code = inspection.problem_code;
        record.connection.problem_message = inspection.problem_message;
        record.connection.recovery_action = inspection.recovery_action;
        record.connection.test_event_supported = inspection.test_event_supported;
        record.connection.updated_at_ms = unix_epoch_ms();
        self.append(
            CONNECTION_UPSERTED,
            &record.connection.connection_id,
            &record,
        )?;
        let connection = record.connection.clone();
        state.connections.insert(key, record);
        Ok(connection)
    }

    pub(crate) fn set_attached_trigger_count(
        &self,
        owner_user_id: &str,
        connection_id: &str,
        attached_trigger_count: u64,
    ) -> Result<EventConnection, DaemonError> {
        let mut state = self.lock_state()?;
        let key = (owner_user_id.to_string(), connection_id.to_string());
        let mut record = state
            .connections
            .get(&key)
            .cloned()
            .ok_or_else(|| registry_error("connection was not found".to_string()))?;
        record.connection.attached_trigger_count = attached_trigger_count;
        if record.connection.status == crate::local::EventConnectionStatus::Ready {
            record.connection.lifecycle_state = if attached_trigger_count == 0 {
                crate::local::EventConnectionLifecycleState::Unused
            } else if record.connection.lifecycle_state
                == crate::local::EventConnectionLifecycleState::Unused
            {
                crate::local::EventConnectionLifecycleState::Connected
            } else {
                record.connection.lifecycle_state
            };
        }
        let connection = record.connection.clone();
        state.connections.insert(key, record);
        Ok(connection)
    }

    pub(crate) fn mark_status(
        &self,
        owner_user_id: &str,
        connection_id: &str,
        status: crate::local::EventConnectionStatus,
    ) -> Result<EventConnection, DaemonError> {
        let current = self
            .get(owner_user_id, connection_id)?
            .ok_or_else(|| registry_error("connection was not found".to_string()))?;
        self.upsert(
            owner_user_id,
            chariox_event_protocol::AegsConnectionSummary {
                generator_id: current.generator_id,
                connection_id: current.connection_id,
                status,
                metadata: current.metadata,
                expires_at_ms: current.expires_at_ms,
                updated_at_ms: unix_epoch_ms(),
            },
        )
    }

    pub(crate) fn remove(
        &self,
        owner_user_id: &str,
        connection_id: &str,
    ) -> Result<bool, DaemonError> {
        let mut state = self.lock_state()?;
        let key = (owner_user_id.to_string(), connection_id.to_string());
        if !state.connections.contains_key(&key) {
            return Ok(false);
        }
        self.append(
            CONNECTION_REMOVED,
            connection_id,
            &RemovedRecord {
                owner_user_id: owner_user_id.to_string(),
                id: connection_id.to_string(),
            },
        )?;
        state.connections.remove(&key);
        Ok(true)
    }

    pub(crate) fn remove_authorizations_for_connection(
        &self,
        owner_user_id: &str,
        connection_id: &str,
    ) -> Result<usize, DaemonError> {
        let mut state = self.lock_state()?;
        let authorization_ids = state
            .authorizations
            .values()
            .filter(|record| {
                record.owner_user_id == owner_user_id
                    && record.authorization.connection_id.as_deref() == Some(connection_id)
            })
            .map(|record| record.authorization.authorization_id.clone())
            .collect::<Vec<_>>();
        for authorization_id in &authorization_ids {
            self.append(
                AUTHORIZATION_REMOVED,
                authorization_id,
                &RemovedRecord {
                    owner_user_id: owner_user_id.to_string(),
                    id: authorization_id.clone(),
                },
            )?;
            state
                .authorizations
                .remove(&(owner_user_id.to_string(), authorization_id.clone()));
        }
        Ok(authorization_ids.len())
    }

    pub(crate) fn start_authorization(
        &self,
        owner_user_id: &str,
        flow: chariox_event_protocol::AegsAuthorizationFlow,
    ) -> Result<EventConnectionAuthorization, DaemonError> {
        if flow
            .connection_id
            .as_deref()
            .is_none_or(|connection_id| connection_id.trim().is_empty())
        {
            return Err(registry_error(
                "AEGS authorization must issue an opaque connection ID".to_string(),
            ));
        }
        let authorization = EventConnectionAuthorization {
            authorization_id: opaque_id("event-authorization"),
            generator_id: flow.generator_id,
            connection_id: flow.connection_id,
            status: flow.status,
            authorization_url: flow.authorization_url,
            user_code: flow.user_code,
            expires_at_ms: flow.expires_at_ms,
            created_at_ms: unix_epoch_ms(),
        };
        let record = StoredAuthorization {
            owner_user_id: owner_user_id.to_string(),
            authorization,
        };
        self.append(
            AUTHORIZATION_UPSERTED,
            &record.authorization.authorization_id,
            &record,
        )?;
        let authorization = record.authorization.clone();
        self.lock_state()?.authorizations.insert(
            (
                owner_user_id.to_string(),
                authorization.authorization_id.clone(),
            ),
            record,
        );
        Ok(authorization)
    }

    pub(crate) fn reconcilable_authorizations(
        &self,
    ) -> Result<Vec<(String, EventConnectionAuthorization)>, DaemonError> {
        let mut state = self.lock_state()?;
        let now_ms = unix_epoch_ms();
        let expired = state
            .authorizations
            .iter()
            .filter(|(_, record)| {
                record
                    .authorization
                    .expires_at_ms
                    .is_some_and(|expires_at_ms| expires_at_ms <= now_ms)
            })
            .map(|(key, record)| (key.clone(), record.authorization.connection_id.clone()))
            .collect::<Vec<_>>();
        for ((owner_user_id, authorization_id), connection_id) in expired {
            if let Some(connection_id) = connection_id {
                self.expire_pending_connection_locked(
                    &mut state,
                    &owner_user_id,
                    &connection_id,
                    now_ms,
                )?;
            }
            self.append(
                AUTHORIZATION_REMOVED,
                &authorization_id,
                &RemovedRecord {
                    owner_user_id: owner_user_id.clone(),
                    id: authorization_id.clone(),
                },
            )?;
            state
                .authorizations
                .remove(&(owner_user_id, authorization_id));
        }
        let orphaned_pending_connections = state
            .connections
            .iter()
            .filter(|(_, record)| {
                record.connection.status == crate::local::EventConnectionStatus::Pending
                    && !state.authorizations.values().any(|authorization| {
                        authorization.owner_user_id == record.owner_user_id
                            && authorization.authorization.connection_id.as_deref()
                                == Some(record.connection.connection_id.as_str())
                    })
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for (owner_user_id, connection_id) in orphaned_pending_connections {
            self.expire_pending_connection_locked(
                &mut state,
                &owner_user_id,
                &connection_id,
                now_ms,
            )?;
        }
        let mut authorizations = state
            .authorizations
            .values()
            .filter(|record| {
                let Some(connection_id) = record.authorization.connection_id.as_deref() else {
                    return true;
                };
                state
                    .connections
                    .get(&(record.owner_user_id.clone(), connection_id.to_string()))
                    .is_none_or(|connection| {
                        connection.connection.status == crate::local::EventConnectionStatus::Pending
                    })
            })
            .map(|record| (record.owner_user_id.clone(), record.authorization.clone()))
            .collect::<Vec<_>>();
        authorizations.sort_by(|left, right| {
            left.1
                .created_at_ms
                .cmp(&right.1.created_at_ms)
                .then_with(|| left.1.authorization_id.cmp(&right.1.authorization_id))
        });
        Ok(authorizations)
    }

    pub(crate) fn authorization(
        &self,
        owner_user_id: &str,
        authorization_id: &str,
    ) -> Result<Option<EventConnectionAuthorization>, DaemonError> {
        let mut state = self.lock_state()?;
        let key = (owner_user_id.to_string(), authorization_id.to_string());
        let Some(record) = state.authorizations.get(&key).cloned() else {
            return Ok(None);
        };
        if record
            .authorization
            .expires_at_ms
            .is_some_and(|expires_at_ms| expires_at_ms <= unix_epoch_ms())
        {
            let now_ms = unix_epoch_ms();
            if let Some(connection_id) = record.authorization.connection_id.as_deref() {
                self.expire_pending_connection_locked(
                    &mut state,
                    owner_user_id,
                    connection_id,
                    now_ms,
                )?;
            }
            self.append(
                AUTHORIZATION_REMOVED,
                authorization_id,
                &RemovedRecord {
                    owner_user_id: owner_user_id.to_string(),
                    id: authorization_id.to_string(),
                },
            )?;
            state.authorizations.remove(&key);
            return Ok(None);
        }
        Ok(Some(record.authorization))
    }

    fn expire_pending_connection_locked(
        &self,
        state: &mut RegistryState,
        owner_user_id: &str,
        connection_id: &str,
        now_ms: u64,
    ) -> Result<bool, DaemonError> {
        let key = (owner_user_id.to_string(), connection_id.to_string());
        let Some(mut record) = state.connections.get(&key).cloned() else {
            return Ok(false);
        };
        if record.connection.status != crate::local::EventConnectionStatus::Pending {
            return Ok(false);
        }
        record.connection.status = crate::local::EventConnectionStatus::Expired;
        record.connection.updated_at_ms = now_ms;
        record.connection.last_validated_at_ms = Some(now_ms);
        self.append(CONNECTION_UPSERTED, connection_id, &record)?;
        state.connections.insert(key, record);
        Ok(true)
    }

    pub(crate) fn update_authorization(
        &self,
        owner_user_id: &str,
        authorization: EventConnectionAuthorization,
    ) -> Result<EventConnectionAuthorization, DaemonError> {
        let key = (
            owner_user_id.to_string(),
            authorization.authorization_id.clone(),
        );
        let mut state = self.lock_state()?;
        if !state.authorizations.contains_key(&key) {
            return Err(registry_error("authorization was not found".to_string()));
        }
        let record = StoredAuthorization {
            owner_user_id: owner_user_id.to_string(),
            authorization,
        };
        self.append(
            AUTHORIZATION_UPSERTED,
            &record.authorization.authorization_id,
            &record,
        )?;
        let authorization = record.authorization.clone();
        state.authorizations.insert(key, record);
        Ok(authorization)
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, RegistryState>, DaemonError> {
        if let Some(error) = self
            .restore_error
            .lock()
            .map_err(|error| registry_error(error.to_string()))?
            .as_ref()
        {
            return Err(registry_error(format!(
                "could not restore durable event connections: {error}"
            )));
        }
        self.state
            .lock()
            .map_err(|error| registry_error(error.to_string()))
    }

    fn restore(&self) -> Result<RegistryState, DaemonError> {
        let mut state = RegistryState::default();
        let mut events = Vec::new();
        for kind in [
            CONNECTION_UPSERTED,
            CONNECTION_REMOVED,
            AUTHORIZATION_UPSERTED,
            AUTHORIZATION_REMOVED,
        ] {
            events.extend(self.durable_state.load_events_by_kind(kind)?);
        }
        events.sort_unstable_by_key(|event| event.sequence);
        for event in events {
            match event.kind.as_str() {
                CONNECTION_UPSERTED => {
                    let mut record: StoredConnection = decode(event.payload)?;
                    if record.connection.lifecycle_state
                        == crate::local::EventConnectionLifecycleState::NotInstalled
                    {
                        record.connection.lifecycle_state =
                            lifecycle_state_for_status(record.connection.status);
                    }
                    state.connections.insert(
                        (
                            record.owner_user_id.clone(),
                            record.connection.connection_id.clone(),
                        ),
                        record,
                    );
                }
                CONNECTION_REMOVED => {
                    let removed: RemovedRecord = decode(event.payload)?;
                    state
                        .connections
                        .remove(&(removed.owner_user_id, removed.id));
                }
                AUTHORIZATION_UPSERTED => {
                    let record: StoredAuthorization = decode(event.payload)?;
                    state.authorizations.insert(
                        (
                            record.owner_user_id.clone(),
                            record.authorization.authorization_id.clone(),
                        ),
                        record,
                    );
                }
                AUTHORIZATION_REMOVED => {
                    let removed: RemovedRecord = decode(event.payload)?;
                    state
                        .authorizations
                        .remove(&(removed.owner_user_id, removed.id));
                }
                _ => {}
            }
        }
        Ok(state)
    }

    fn append<T: Serialize>(
        &self,
        kind: &str,
        subject_id: &str,
        value: &T,
    ) -> Result<(), DaemonError> {
        let payload =
            serde_json::to_value(value).map_err(|error| registry_error(error.to_string()))?;
        self.durable_state
            .append_event(kind, Some(subject_id.to_string()), payload)?;
        Ok(())
    }
}

// Durable registry decoding follows the same shared error contract as the registry operations.
#[allow(clippy::result_large_err)]
fn decode<T: for<'de> Deserialize<'de>>(value: serde_json::Value) -> Result<T, DaemonError> {
    serde_json::from_value(value).map_err(|error| registry_error(error.to_string()))
}

fn registry_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "event_connection.registry",
        message,
    }
}

fn opaque_id(prefix: &str) -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let suffix = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{prefix}-{suffix}")
}

fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn lifecycle_state_for_status(
    status: crate::local::EventConnectionStatus,
) -> crate::local::EventConnectionLifecycleState {
    match status {
        crate::local::EventConnectionStatus::Pending => {
            crate::local::EventConnectionLifecycleState::AuthorizationRequired
        }
        crate::local::EventConnectionStatus::Ready => {
            crate::local::EventConnectionLifecycleState::Connected
        }
        crate::local::EventConnectionStatus::Expired => {
            crate::local::EventConnectionLifecycleState::ReauthorizationRequired
        }
        crate::local::EventConnectionStatus::Revoked => {
            crate::local::EventConnectionLifecycleState::Disconnected
        }
        crate::local::EventConnectionStatus::Unavailable => {
            crate::local::EventConnectionLifecycleState::AegsUnavailable
        }
        crate::local::EventConnectionStatus::Error => {
            crate::local::EventConnectionLifecycleState::Degraded
        }
    }
}

fn status_for_lifecycle(
    state: crate::local::EventConnectionLifecycleState,
) -> crate::local::EventConnectionStatus {
    match state {
        crate::local::EventConnectionLifecycleState::AuthorizationRequired
        | crate::local::EventConnectionLifecycleState::Connecting => {
            crate::local::EventConnectionStatus::Pending
        }
        crate::local::EventConnectionLifecycleState::Connected
        | crate::local::EventConnectionLifecycleState::ConnectedRestricted
        | crate::local::EventConnectionLifecycleState::Unused => {
            crate::local::EventConnectionStatus::Ready
        }
        crate::local::EventConnectionLifecycleState::ReauthorizationRequired => {
            crate::local::EventConnectionStatus::Expired
        }
        crate::local::EventConnectionLifecycleState::ProviderUnreachable
        | crate::local::EventConnectionLifecycleState::AegsUnavailable => {
            crate::local::EventConnectionStatus::Unavailable
        }
        crate::local::EventConnectionLifecycleState::Disconnecting
        | crate::local::EventConnectionLifecycleState::Disconnected => {
            crate::local::EventConnectionStatus::Revoked
        }
        crate::local::EventConnectionLifecycleState::NotInstalled
        | crate::local::EventConnectionLifecycleState::Degraded => {
            crate::local::EventConnectionStatus::Error
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_event_protocol::AegsConnectionStatus;
    use serde_json::json;

    #[test]
    fn registry_is_owner_scoped_and_restores_after_reopen() {
        let root = std::env::temp_dir().join(opaque_id("chariox-event-connection-test"));
        let path = root.join("state.sqlite3");
        let store = DurableKernelStateStore::open(path.clone()).unwrap();
        let registry = EventConnectionRegistry::new(store);
        let missing_connection_id = registry
            .start_authorization(
                "user-a",
                chariox_event_protocol::AegsAuthorizationFlow {
                    generator_id: "dev.chariox.github".to_string(),
                    status: "user_action_required".to_string(),
                    connection_id: None,
                    authorization_url: Some("https://example.test/authorize".to_string()),
                    user_code: None,
                    expires_at_ms: None,
                },
            )
            .expect_err("kernel reconciliation requires an opaque connection ID");
        assert!(missing_connection_id
            .to_string()
            .contains("must issue an opaque connection ID"));
        let expired = registry
            .start_authorization(
                "user-a",
                chariox_event_protocol::AegsAuthorizationFlow {
                    generator_id: "dev.chariox.github".to_string(),
                    status: "user_action_required".to_string(),
                    connection_id: Some("expired-connection".to_string()),
                    authorization_url: Some("https://example.test/authorize".to_string()),
                    user_code: None,
                    expires_at_ms: Some(1),
                },
            )
            .unwrap();
        assert!(registry.reconcilable_authorizations().unwrap().is_empty());
        assert!(registry
            .authorization("user-a", &expired.authorization_id)
            .unwrap()
            .is_none());
        registry
            .upsert(
                "user-a",
                chariox_event_protocol::AegsConnectionSummary {
                    generator_id: "dev.chariox.github".to_string(),
                    connection_id: "github-connection-1".to_string(),
                    status: AegsConnectionStatus::Ready,
                    metadata: json!({"account": "chariox"}),
                    expires_at_ms: None,
                    updated_at_ms: 42,
                },
            )
            .unwrap();
        assert_eq!(registry.list("user-a", None).unwrap().len(), 1);
        assert!(registry.list("user-b", None).unwrap().is_empty());
        drop(registry);

        let restored =
            EventConnectionRegistry::new(DurableKernelStateStore::open(path.clone()).unwrap());
        assert_eq!(restored.list("user-a", None).unwrap().len(), 1);
        assert!(restored
            .get("user-b", "github-connection-1")
            .unwrap()
            .is_none());
        let authorization = restored
            .start_authorization(
                "user-a",
                chariox_event_protocol::AegsAuthorizationFlow {
                    generator_id: "dev.chariox.github".to_string(),
                    status: "pending".to_string(),
                    connection_id: Some("github-connection-1".to_string()),
                    authorization_url: Some("https://example.test/authorize".to_string()),
                    user_code: None,
                    expires_at_ms: None,
                },
            )
            .unwrap();
        assert_eq!(
            restored
                .remove_authorizations_for_connection("user-a", "github-connection-1")
                .unwrap(),
            1
        );
        assert!(restored
            .authorization("user-a", &authorization.authorization_id)
            .unwrap()
            .is_none());
        assert!(restored.remove("user-a", "github-connection-1").unwrap());
        drop(restored);

        let removed = EventConnectionRegistry::new(DurableKernelStateStore::open(path).unwrap());
        assert!(removed.list("user-a", None).unwrap().is_empty());
        assert!(removed
            .authorization("user-a", &authorization.authorization_id)
            .unwrap()
            .is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn expired_authorizations_and_orphaned_pending_connections_require_reauthorization() {
        let root = std::env::temp_dir().join(opaque_id("chariox-event-authorization-expiry-test"));
        let path = root.join("state.sqlite3");
        let registry = EventConnectionRegistry::new(
            DurableKernelStateStore::open(path.clone()).expect("store should initialize"),
        );
        registry
            .upsert(
                "user-a",
                chariox_event_protocol::AegsConnectionSummary {
                    generator_id: "dev.chariox.github".to_string(),
                    connection_id: "expired-connection".to_string(),
                    status: AegsConnectionStatus::Pending,
                    metadata: json!({"account": "chariox"}),
                    expires_at_ms: None,
                    updated_at_ms: 1,
                },
            )
            .expect("pending connection should be stored");
        let authorization = registry
            .start_authorization(
                "user-a",
                chariox_event_protocol::AegsAuthorizationFlow {
                    generator_id: "dev.chariox.github".to_string(),
                    status: "user_action_required".to_string(),
                    connection_id: Some("expired-connection".to_string()),
                    authorization_url: Some("https://example.test/authorize".to_string()),
                    user_code: None,
                    expires_at_ms: Some(1),
                },
            )
            .expect("authorization should be stored before expiry reconciliation");

        assert!(registry
            .reconcilable_authorizations()
            .expect("expiry reconciliation should succeed")
            .is_empty());
        assert!(registry
            .authorization("user-a", &authorization.authorization_id)
            .expect("authorization lookup should succeed")
            .is_none());
        assert_eq!(
            registry
                .get("user-a", "expired-connection")
                .expect("connection lookup should succeed")
                .expect("connection should remain available for reconnect")
                .status,
            AegsConnectionStatus::Expired,
        );

        registry
            .upsert(
                "user-a",
                chariox_event_protocol::AegsConnectionSummary {
                    generator_id: "dev.chariox.github".to_string(),
                    connection_id: "orphaned-connection".to_string(),
                    status: AegsConnectionStatus::Pending,
                    metadata: json!({}),
                    expires_at_ms: None,
                    updated_at_ms: 2,
                },
            )
            .expect("orphaned pending connection should be stored");
        registry
            .reconcilable_authorizations()
            .expect("orphan reconciliation should succeed");
        assert_eq!(
            registry
                .get("user-a", "orphaned-connection")
                .expect("connection lookup should succeed")
                .expect("orphan should remain reconnectable")
                .status,
            AegsConnectionStatus::Expired,
        );
        drop(registry);

        let restored = EventConnectionRegistry::new(
            DurableKernelStateStore::open(path).expect("store should reopen"),
        );
        for connection_id in ["expired-connection", "orphaned-connection"] {
            assert_eq!(
                restored
                    .get("user-a", connection_id)
                    .expect("restored lookup should succeed")
                    .expect("expired connection should restore")
                    .status,
                AegsConnectionStatus::Expired,
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_restore_does_not_decode_unrelated_durable_events() {
        let root = std::env::temp_dir().join(opaque_id("chariox-event-connection-indexed-test"));
        let path = root.join("state.sqlite3");
        drop(DurableKernelStateStore::open(path.clone()).expect("store should initialize"));
        let connection = rusqlite::Connection::open(&path).expect("database should open");
        connection
            .execute(
                "INSERT INTO durable_state_events (
                    event_id, kind, subject_id, timestamp_ms, payload_json
                 ) VALUES ('unrelated-event', 'session.updated', 'session-1', 1, 'not-json')",
                [],
            )
            .expect("unrelated event should insert");
        drop(connection);

        let registry = EventConnectionRegistry::new(
            DurableKernelStateStore::open(path.clone()).expect("store should reopen"),
        );
        assert!(registry
            .list("user-a", None)
            .expect("indexed restore should ignore unrelated events")
            .is_empty());

        let _ = std::fs::remove_dir_all(root);
    }
}
