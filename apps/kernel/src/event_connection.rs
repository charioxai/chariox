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
        summary: arroba_event_protocol::AegsConnectionSummary,
    ) -> Result<EventConnection, DaemonError> {
        let now_ms = unix_epoch_ms();
        let key = (owner_user_id.to_string(), summary.connection_id.clone());
        let mut state = self.lock_state()?;
        let created_at_ms = state
            .connections
            .get(&key)
            .map(|record| record.connection.created_at_ms)
            .unwrap_or(now_ms);
        let record = StoredConnection {
            owner_user_id: owner_user_id.to_string(),
            connection: EventConnection {
                generator_id: summary.generator_id,
                connection_id: summary.connection_id,
                status: summary.status,
                metadata: summary.metadata,
                expires_at_ms: summary.expires_at_ms,
                created_at_ms,
                updated_at_ms: summary.updated_at_ms,
                last_validated_at_ms: Some(now_ms),
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

    pub(crate) fn start_authorization(
        &self,
        owner_user_id: &str,
        flow: arroba_event_protocol::AegsAuthorizationFlow,
    ) -> Result<EventConnectionAuthorization, DaemonError> {
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
        for event in self.durable_state.load_events_after(0)? {
            match event.kind.as_str() {
                CONNECTION_UPSERTED => {
                    let record: StoredConnection = decode(event.payload)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use arroba_event_protocol::AegsConnectionStatus;
    use serde_json::json;

    #[test]
    fn registry_is_owner_scoped_and_restores_after_reopen() {
        let root = std::env::temp_dir().join(opaque_id("arroba-event-connection-test"));
        let path = root.join("state.sqlite3");
        let store = DurableKernelStateStore::open(path.clone()).unwrap();
        let registry = EventConnectionRegistry::new(store);
        registry
            .upsert(
                "user-a",
                arroba_event_protocol::AegsConnectionSummary {
                    generator_id: "dev.arroba.github".to_string(),
                    connection_id: "github-connection-1".to_string(),
                    status: AegsConnectionStatus::Ready,
                    metadata: json!({"account": "arroba"}),
                    expires_at_ms: None,
                    updated_at_ms: 42,
                },
            )
            .unwrap();
        assert_eq!(registry.list("user-a", None).unwrap().len(), 1);
        assert!(registry.list("user-b", None).unwrap().is_empty());
        drop(registry);

        let restored = EventConnectionRegistry::new(DurableKernelStateStore::open(path).unwrap());
        assert_eq!(restored.list("user-a", None).unwrap().len(), 1);
        assert!(restored
            .get("user-b", "github-connection-1")
            .unwrap()
            .is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
