use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::durable_state::{DurableKernelStateStore, DurableStateEvent};

use super::{
    EnvironmentAction, EnvironmentTab, InputOwnership, InputTarget, RoomEnvironmentSnapshot,
};

pub(crate) const SAVED_ROOM_GENERATION_SCHEMA_VERSION: u32 = 1;
pub(crate) const SAVED_ROOM_GENERATION_PREPARE_EVENT_KIND: &str = "saved_room_generation.prepare";
pub(crate) const SAVED_ROOM_GENERATION_COMMIT_EVENT_KIND: &str = "saved_room_generation.commit";
pub(crate) const SAVED_ROOM_GENERATION_ROLLBACK_EVENT_KIND: &str = "saved_room_generation.rollback";

const DIGEST_PREFIX: &str = "sha256:";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum SavedRoomGenerationError {
    #[error("saved Room generation is missing required class `{0}`")]
    MissingClass(&'static str),
    #[error("saved Room generation field `{field}` is invalid: {message}")]
    InvalidField {
        field: &'static str,
        message: String,
    },
    #[error(
        "saved Room generation `{class}` digest mismatch: expected {expected}, computed {actual}"
    )]
    DigestMismatch {
        class: &'static str,
        expected: String,
        actual: String,
    },
    #[error("saved Room generation crosses a generation boundary: {0}")]
    CrossGeneration(String),
    #[error("saved Room generation journal `{operation}` failed: {message}")]
    Journal {
        operation: &'static str,
        message: String,
    },
    #[error("saved Room generation encoding failed: {0}")]
    Encoding(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedRoomIdentity {
    pub(crate) room_id: String,
    pub(crate) environment_id: String,
    pub(crate) runtime_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedTabRegistry {
    pub(crate) tabs: Vec<EnvironmentTab>,
    pub(crate) focused_tab_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedActionLedger {
    pub(crate) actions: Vec<EnvironmentAction>,
    pub(crate) ownership: Vec<InputOwnership>,
    pub(crate) next_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedPermissionDecision {
    pub(crate) tab_id: String,
    pub(crate) document_revision: u64,
    pub(crate) permission: String,
    pub(crate) setting: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedSourceIdentity {
    pub(crate) commit: String,
    pub(crate) tree: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedRuntimeImageIdentity {
    pub(crate) digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedRoomGenerationClassDigests {
    pub(crate) room_identity: String,
    pub(crate) tab_registry: String,
    pub(crate) action_ledger: String,
    pub(crate) permissions: String,
    pub(crate) source: String,
    pub(crate) runtime_image: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedRoomGenerationRef {
    pub(crate) generation_id: String,
    pub(crate) generation_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedRoomGeneration {
    pub(crate) schema_version: u32,
    pub(crate) generation_id: String,
    pub(crate) room: SavedRoomIdentity,
    pub(crate) tabs: SavedTabRegistry,
    pub(crate) action_ledger: SavedActionLedger,
    pub(crate) permissions: Vec<SavedPermissionDecision>,
    pub(crate) source: SavedSourceIdentity,
    pub(crate) runtime_image: SavedRuntimeImageIdentity,
    pub(crate) class_digests: SavedRoomGenerationClassDigests,
    pub(crate) previous_generation: Option<SavedRoomGenerationRef>,
    pub(crate) generation_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SavedRoomRestorePlan {
    generation: SavedRoomGeneration,
}

impl SavedRoomRestorePlan {
    pub(crate) fn generation(&self) -> &SavedRoomGeneration {
        &self.generation
    }
}

/// Boundary for the physical Browser Controller/profile and other runtime owners.
///
/// The kernel validates the complete plan before calling this boundary. No adapter is
/// implemented here until the owner can prepare and commit its physical state atomically.
#[allow(dead_code)]
pub(crate) trait SavedRoomGenerationRestoreAdapter {
    type Error;

    fn prepare_restore(&mut self, plan: &SavedRoomRestorePlan) -> Result<(), Self::Error>;
    fn commit_restore(&mut self, plan: SavedRoomRestorePlan) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedSavedRoomGeneration {
    generation: SavedRoomGeneration,
    prepare_event_sequence: u64,
}

impl PreparedSavedRoomGeneration {
    pub(crate) fn generation(&self) -> &SavedRoomGeneration {
        &self.generation
    }

    pub(crate) fn prepare_event_sequence(&self) -> u64 {
        self.prepare_event_sequence
    }
}

impl SavedRoomGeneration {
    pub(crate) fn capture(
        generation_id: impl Into<String>,
        snapshot: RoomEnvironmentSnapshot,
        permissions: Vec<SavedPermissionDecision>,
        source: SavedSourceIdentity,
        runtime_image: SavedRuntimeImageIdentity,
        previous_generation: Option<SavedRoomGenerationRef>,
    ) -> Result<Self, SavedRoomGenerationError> {
        let next_sequence = snapshot
            .actions
            .iter()
            .map(|action| action.sequence)
            .max()
            .unwrap_or_default()
            .saturating_add(1)
            .max(1);
        let mut generation = Self {
            schema_version: SAVED_ROOM_GENERATION_SCHEMA_VERSION,
            generation_id: generation_id.into(),
            room: SavedRoomIdentity {
                room_id: snapshot.session_id,
                environment_id: snapshot.environment_id,
                runtime_generation: snapshot.runtime_generation,
            },
            tabs: SavedTabRegistry {
                tabs: snapshot.tabs,
                focused_tab_id: snapshot.focused_tab_id,
            },
            action_ledger: SavedActionLedger {
                actions: snapshot.actions,
                ownership: snapshot.input_ownership,
                next_sequence,
            },
            permissions,
            source,
            runtime_image,
            class_digests: SavedRoomGenerationClassDigests {
                room_identity: String::new(),
                tab_registry: String::new(),
                action_ledger: String::new(),
                permissions: String::new(),
                source: String::new(),
                runtime_image: String::new(),
            },
            previous_generation,
            generation_digest: String::new(),
        };
        generation.class_digests = generation.compute_class_digests()?;
        generation.generation_digest = generation.compute_generation_digest()?;
        generation.validate()?;
        Ok(generation)
    }

    pub(crate) fn validate(&self) -> Result<(), SavedRoomGenerationError> {
        if self.schema_version != SAVED_ROOM_GENERATION_SCHEMA_VERSION {
            return Err(SavedRoomGenerationError::InvalidField {
                field: "schema_version",
                message: format!(
                    "expected {}, got {}",
                    SAVED_ROOM_GENERATION_SCHEMA_VERSION, self.schema_version
                ),
            });
        }
        require_identifier("generation_id", &self.generation_id)?;
        require_identifier("room.room_id", &self.room.room_id)?;
        require_identifier("room.environment_id", &self.room.environment_id)?;
        if self.room.runtime_generation == 0 {
            return Err(SavedRoomGenerationError::InvalidField {
                field: "room.runtime_generation",
                message: "must be non-zero".to_string(),
            });
        }
        validate_tabs(&self.tabs)?;
        validate_actions(&self.action_ledger, &self.tabs)?;
        validate_permissions(&self.permissions, &self.tabs)?;
        validate_source(&self.source)?;
        validate_runtime_image(&self.runtime_image)?;
        validate_generation_ref(self.previous_generation.as_ref())?;
        validate_digest("generation_digest", &self.generation_digest)?;

        let computed = self.compute_class_digests()?;
        ensure_digest_matches(
            "room_identity",
            &self.class_digests.room_identity,
            &computed.room_identity,
        )?;
        ensure_digest_matches(
            "tab_registry",
            &self.class_digests.tab_registry,
            &computed.tab_registry,
        )?;
        ensure_digest_matches(
            "action_ledger",
            &self.class_digests.action_ledger,
            &computed.action_ledger,
        )?;
        ensure_digest_matches(
            "permissions",
            &self.class_digests.permissions,
            &computed.permissions,
        )?;
        ensure_digest_matches("source", &self.class_digests.source, &computed.source)?;
        ensure_digest_matches(
            "runtime_image",
            &self.class_digests.runtime_image,
            &computed.runtime_image,
        )?;
        let computed_generation_digest = self.compute_generation_digest()?;
        if self.generation_digest != computed_generation_digest {
            return Err(SavedRoomGenerationError::DigestMismatch {
                class: "generation",
                expected: self.generation_digest.clone(),
                actual: computed_generation_digest,
            });
        }
        Ok(())
    }

    pub(crate) fn validate_against(
        &self,
        previous: Option<&SavedRoomGeneration>,
    ) -> Result<(), SavedRoomGenerationError> {
        self.validate()?;
        match (&self.previous_generation, previous) {
            (None, None) => Ok(()),
            (Some(reference), Some(previous)) => {
                if previous.room.room_id != self.room.room_id
                    || previous.room.environment_id != self.room.environment_id
                {
                    return Err(SavedRoomGenerationError::CrossGeneration(
                        "previous generation belongs to a different Room/environment".to_string(),
                    ));
                }
                if reference.generation_id != previous.generation_id
                    || reference.generation_digest != previous.generation_digest
                {
                    return Err(SavedRoomGenerationError::CrossGeneration(
                        "previous generation reference does not match the authoritative generation"
                            .to_string(),
                    ));
                }
                Ok(())
            }
            (None, Some(_)) => Err(SavedRoomGenerationError::CrossGeneration(
                "new generation omitted the authoritative previous generation".to_string(),
            )),
            (Some(_), None) => Err(SavedRoomGenerationError::CrossGeneration(
                "generation references a previous generation that is not authoritative".to_string(),
            )),
        }
    }

    pub(crate) fn restore_plan(&self) -> Result<SavedRoomRestorePlan, SavedRoomGenerationError> {
        self.validate()?;
        Ok(SavedRoomRestorePlan {
            generation: self.clone(),
        })
    }

    pub(crate) fn generation_ref(&self) -> SavedRoomGenerationRef {
        SavedRoomGenerationRef {
            generation_id: self.generation_id.clone(),
            generation_digest: self.generation_digest.clone(),
        }
    }

    fn compute_class_digests(
        &self,
    ) -> Result<SavedRoomGenerationClassDigests, SavedRoomGenerationError> {
        Ok(SavedRoomGenerationClassDigests {
            room_identity: digest_of(&self.room)?,
            tab_registry: digest_of(&self.tabs)?,
            action_ledger: digest_of(&self.action_ledger)?,
            permissions: digest_of(&self.permissions)?,
            source: digest_of(&self.source)?,
            runtime_image: digest_of(&self.runtime_image)?,
        })
    }

    fn compute_generation_digest(&self) -> Result<String, SavedRoomGenerationError> {
        digest_of(&GenerationDigestInput {
            schema_version: self.schema_version,
            generation_id: self.generation_id.clone(),
            room: self.room.clone(),
            tabs: self.tabs.clone(),
            action_ledger: self.action_ledger.clone(),
            permissions: self.permissions.clone(),
            source: self.source.clone(),
            runtime_image: self.runtime_image.clone(),
            class_digests: self.class_digests.clone(),
            previous_generation: self.previous_generation.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
struct GenerationDigestInput {
    schema_version: u32,
    generation_id: String,
    room: SavedRoomIdentity,
    tabs: SavedTabRegistry,
    action_ledger: SavedActionLedger,
    permissions: Vec<SavedPermissionDecision>,
    source: SavedSourceIdentity,
    runtime_image: SavedRuntimeImageIdentity,
    class_digests: SavedRoomGenerationClassDigests,
    previous_generation: Option<SavedRoomGenerationRef>,
}

pub(crate) struct SavedRoomGenerationJournal<'a> {
    store: &'a DurableKernelStateStore,
}

impl<'a> SavedRoomGenerationJournal<'a> {
    pub(crate) fn new(store: &'a DurableKernelStateStore) -> Self {
        Self { store }
    }

    pub(crate) fn latest_committed(
        &self,
        room_id: &str,
        environment_id: &str,
    ) -> Result<Option<SavedRoomGeneration>, SavedRoomGenerationError> {
        self.latest_committed_before(room_id, environment_id, u64::MAX)
    }

    pub(crate) fn latest_committed_before(
        &self,
        room_id: &str,
        environment_id: &str,
        sequence: u64,
    ) -> Result<Option<SavedRoomGeneration>, SavedRoomGenerationError> {
        let events = self
            .store
            .load_events_by_kind(SAVED_ROOM_GENERATION_COMMIT_EVENT_KIND)
            .map_err(|error| journal_error("load_committed_generations", error.to_string()))?;
        let prepare_events = self
            .store
            .load_events_by_kind(SAVED_ROOM_GENERATION_PREPARE_EVENT_KIND)
            .map_err(|error| journal_error("load_prepared_generations", error.to_string()))?;
        let mut committed = Vec::new();
        for event in events {
            if event.sequence > sequence {
                break;
            }
            let generation = validate_commit_event(&event, &prepare_events)?;
            if generation.room.room_id != room_id
                || generation.room.environment_id != environment_id
            {
                continue;
            }
            committed.push(generation);
        }

        let mut previous: Option<SavedRoomGeneration> = None;
        for generation in committed {
            generation.validate_against(previous.as_ref())?;
            previous = Some(generation);
        }
        Ok(previous)
    }

    pub(crate) fn validate_commit_event(
        &self,
        event: &DurableStateEvent,
    ) -> Result<SavedRoomGeneration, SavedRoomGenerationError> {
        let prepare_events = self
            .store
            .load_events_by_kind(SAVED_ROOM_GENERATION_PREPARE_EVENT_KIND)
            .map_err(|error| journal_error("validate_commit_event", error.to_string()))?;
        let generation = validate_commit_event(event, &prepare_events)?;
        let committed_events = self
            .store
            .load_events_by_kind(SAVED_ROOM_GENERATION_COMMIT_EVENT_KIND)
            .map_err(|error| journal_error("validate_commit_event", error.to_string()))?;
        let mut previous: Option<SavedRoomGeneration> = None;
        for prior_event in committed_events {
            if prior_event.sequence >= event.sequence {
                break;
            }
            let prior = validate_commit_event(&prior_event, &prepare_events)?;
            if prior.room.room_id == generation.room.room_id
                && prior.room.environment_id == generation.room.environment_id
            {
                prior.validate_against(previous.as_ref())?;
                previous = Some(prior);
            }
        }
        generation.validate_against(previous.as_ref())?;
        Ok(generation)
    }

    pub(crate) fn prepare(
        &self,
        generation: SavedRoomGeneration,
    ) -> Result<PreparedSavedRoomGeneration, SavedRoomGenerationError> {
        let previous =
            self.latest_committed(&generation.room.room_id, &generation.room.environment_id)?;
        generation.validate_against(previous.as_ref())?;
        let event = self
            .store
            .append_event(
                SAVED_ROOM_GENERATION_PREPARE_EVENT_KIND,
                Some(generation.room.room_id.clone()),
                serde_json::json!({ "generation": generation }),
            )
            .map_err(|error| journal_error("prepare", error.to_string()))?;
        Ok(PreparedSavedRoomGeneration {
            generation,
            prepare_event_sequence: event.sequence,
        })
    }

    pub(crate) fn commit(
        &self,
        prepared: PreparedSavedRoomGeneration,
    ) -> Result<SavedRoomGeneration, SavedRoomGenerationError> {
        let prepare_events = self
            .store
            .load_events_by_kind(SAVED_ROOM_GENERATION_PREPARE_EVENT_KIND)
            .map_err(|error| journal_error("commit", error.to_string()))?;
        validate_prepare_event(
            prepared.prepare_event_sequence,
            &prepared.generation,
            &prepare_events,
        )?;
        let previous = self.latest_committed(
            &prepared.generation.room.room_id,
            &prepared.generation.room.environment_id,
        )?;
        prepared.generation.validate_against(previous.as_ref())?;
        self.store
            .append_event(
                SAVED_ROOM_GENERATION_COMMIT_EVENT_KIND,
                Some(prepared.generation.room.room_id.clone()),
                serde_json::json!({
                    "generation": prepared.generation,
                    "prepare_event_sequence": prepared.prepare_event_sequence,
                }),
            )
            .map_err(|error| journal_error("commit", error.to_string()))?;
        Ok(prepared.generation)
    }

    pub(crate) fn rollback(
        &self,
        prepared: &PreparedSavedRoomGeneration,
        reason: impl Into<String>,
    ) -> Result<(), SavedRoomGenerationError> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(SavedRoomGenerationError::InvalidField {
                field: "rollback.reason",
                message: "must not be empty".to_string(),
            });
        }
        self.store
            .append_event(
                SAVED_ROOM_GENERATION_ROLLBACK_EVENT_KIND,
                Some(prepared.generation.room.room_id.clone()),
                serde_json::json!({
                    "generation_id": prepared.generation.generation_id,
                    "generation_digest": prepared.generation.generation_digest,
                    "prepare_event_sequence": prepared.prepare_event_sequence,
                    "reason": reason,
                }),
            )
            .map_err(|error| journal_error("rollback", error.to_string()))?;
        Ok(())
    }
}

fn decode_generation_event(
    payload: &Value,
    operation: &'static str,
) -> Result<SavedRoomGeneration, SavedRoomGenerationError> {
    let generation = payload
        .get("generation")
        .cloned()
        .ok_or(SavedRoomGenerationError::MissingClass("generation"))?;
    let generation: SavedRoomGeneration = serde_json::from_value(generation)
        .map_err(|error| journal_error(operation, error.to_string()))?;
    generation.validate()?;
    Ok(generation)
}

fn validate_commit_event(
    event: &DurableStateEvent,
    prepare_events: &[DurableStateEvent],
) -> Result<SavedRoomGeneration, SavedRoomGenerationError> {
    let generation = decode_generation_event(&event.payload, "load_committed_generations")?;
    let prepare_event_sequence = event
        .payload
        .get("prepare_event_sequence")
        .and_then(Value::as_u64)
        .ok_or(SavedRoomGenerationError::MissingClass(
            "prepare_event_sequence",
        ))?;
    if prepare_event_sequence >= event.sequence {
        return Err(SavedRoomGenerationError::CrossGeneration(format!(
            "generation `{}` prepare event is not before its commit",
            generation.generation_id
        )));
    }
    validate_prepare_event(prepare_event_sequence, &generation, prepare_events)?;
    Ok(generation)
}

fn validate_prepare_event(
    prepare_event_sequence: u64,
    expected: &SavedRoomGeneration,
    prepare_events: &[DurableStateEvent],
) -> Result<(), SavedRoomGenerationError> {
    let prepare_event = prepare_events
        .iter()
        .find(|prepare| prepare.sequence == prepare_event_sequence)
        .ok_or_else(|| {
            SavedRoomGenerationError::CrossGeneration(format!(
                "generation `{}` has no matching prepare event",
                expected.generation_id
            ))
        })?;
    let prepared_generation =
        decode_generation_event(&prepare_event.payload, "load_prepared_generations")?;
    if &prepared_generation != expected {
        return Err(SavedRoomGenerationError::CrossGeneration(format!(
            "generation `{}` does not match its prepare event",
            expected.generation_id
        )));
    }
    Ok(())
}

fn validate_tabs(registry: &SavedTabRegistry) -> Result<(), SavedRoomGenerationError> {
    let mut ids = BTreeSet::new();
    let mut focused = 0usize;
    for tab in &registry.tabs {
        require_identifier("tabs.tab_id", &tab.tab_id)?;
        if !ids.insert(tab.tab_id.clone()) {
            return Err(SavedRoomGenerationError::InvalidField {
                field: "tabs.tab_id",
                message: format!("duplicate tab id `{}`", tab.tab_id),
            });
        }
        if tab.document_revision == 0 {
            return Err(SavedRoomGenerationError::InvalidField {
                field: "tabs.document_revision",
                message: format!("tab `{}` has zero document revision", tab.tab_id),
            });
        }
        if tab.focused {
            focused += 1;
        }
    }
    match (&registry.focused_tab_id, focused) {
        (Some(tab_id), 1) if ids.contains(tab_id) => Ok(()),
        (None, 0) => Ok(()),
        (Some(tab_id), _) if !ids.contains(tab_id) => Err(SavedRoomGenerationError::InvalidField {
            field: "tabs.focused_tab_id",
            message: format!("focused tab `{tab_id}` is not in the registry"),
        }),
        _ => Err(SavedRoomGenerationError::InvalidField {
            field: "tabs.focused_tab_id",
            message: "focused tab id and tab focus flags disagree".to_string(),
        }),
    }
}

fn validate_actions(
    ledger: &SavedActionLedger,
    tabs: &SavedTabRegistry,
) -> Result<(), SavedRoomGenerationError> {
    if ledger.next_sequence == 0 {
        return Err(SavedRoomGenerationError::InvalidField {
            field: "action_ledger.next_sequence",
            message: "must be non-zero".to_string(),
        });
    }
    let mut sequences = BTreeSet::new();
    let mut action_ids = BTreeSet::new();
    let mut maximum = 0;
    for action in &ledger.actions {
        require_identifier("action_ledger.action_id", &action.action_id)?;
        if !action_ids.insert(action.action_id.clone()) {
            return Err(SavedRoomGenerationError::InvalidField {
                field: "action_ledger.action_id",
                message: format!("duplicate action id `{}`", action.action_id),
            });
        }
        if action.sequence == 0 || !sequences.insert(action.sequence) {
            return Err(SavedRoomGenerationError::InvalidField {
                field: "action_ledger.sequence",
                message: format!("invalid or duplicate sequence {}", action.sequence),
            });
        }
        for target in &action.targets {
            if let InputTarget::BrowserTab(tab_id) = target {
                if !tabs.tabs.iter().any(|tab| tab.tab_id == *tab_id) {
                    return Err(SavedRoomGenerationError::InvalidField {
                        field: "action_ledger.targets",
                        message: format!("action references unknown tab `{tab_id}`"),
                    });
                }
            }
        }
        maximum = maximum.max(action.sequence);
    }
    if ledger.next_sequence <= maximum {
        return Err(SavedRoomGenerationError::InvalidField {
            field: "action_ledger.next_sequence",
            message: format!("must be greater than maximum sequence {maximum}"),
        });
    }
    let mut owned_targets = BTreeSet::new();
    for ownership in &ledger.ownership {
        if !owned_targets.insert(ownership.target.clone()) {
            return Err(SavedRoomGenerationError::InvalidField {
                field: "action_ledger.ownership",
                message: "duplicate owned input target".to_string(),
            });
        }
        require_identifier("action_ledger.ownership.actor_id", &ownership.actor_id)?;
        if let InputTarget::BrowserTab(tab_id) = &ownership.target {
            require_identifier("action_ledger.ownership.tab_id", tab_id)?;
            if !tabs.tabs.iter().any(|tab| tab.tab_id == *tab_id) {
                return Err(SavedRoomGenerationError::InvalidField {
                    field: "action_ledger.ownership",
                    message: format!("ownership references unknown tab `{tab_id}`"),
                });
            }
        }
    }
    Ok(())
}

fn validate_permissions(
    permissions: &[SavedPermissionDecision],
    tabs: &SavedTabRegistry,
) -> Result<(), SavedRoomGenerationError> {
    let tab_ids = tabs
        .tabs
        .iter()
        .map(|tab| tab.tab_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut decisions = BTreeSet::new();
    for decision in permissions {
        require_identifier("permissions.tab_id", &decision.tab_id)?;
        if !tab_ids.contains(decision.tab_id.as_str()) {
            return Err(SavedRoomGenerationError::InvalidField {
                field: "permissions.tab_id",
                message: format!("decision references unknown tab `{}`", decision.tab_id),
            });
        }
        require_identifier("permissions.permission", &decision.permission)?;
        require_identifier("permissions.setting", &decision.setting)?;
        if !decisions.insert((
            decision.tab_id.clone(),
            decision.document_revision,
            decision.permission.clone(),
        )) {
            return Err(SavedRoomGenerationError::InvalidField {
                field: "permissions",
                message: "duplicate permission decision".to_string(),
            });
        }
    }
    Ok(())
}

fn validate_source(source: &SavedSourceIdentity) -> Result<(), SavedRoomGenerationError> {
    validate_hex("source.commit", &source.commit, 40)?;
    validate_hex("source.tree", &source.tree, 40)
}

fn validate_runtime_image(
    runtime_image: &SavedRuntimeImageIdentity,
) -> Result<(), SavedRoomGenerationError> {
    validate_digest("runtime_image.digest", &runtime_image.digest)
}

fn validate_generation_ref(
    reference: Option<&SavedRoomGenerationRef>,
) -> Result<(), SavedRoomGenerationError> {
    if let Some(reference) = reference {
        require_identifier(
            "previous_generation.generation_id",
            &reference.generation_id,
        )?;
        validate_digest(
            "previous_generation.generation_digest",
            &reference.generation_digest,
        )?;
    }
    Ok(())
}

fn require_identifier(field: &'static str, value: &str) -> Result<(), SavedRoomGenerationError> {
    if value.trim().is_empty() || value != value.trim() {
        return Err(SavedRoomGenerationError::InvalidField {
            field,
            message: "must be non-empty and already trimmed".to_string(),
        });
    }
    Ok(())
}

fn validate_hex(
    field: &'static str,
    value: &str,
    length: usize,
) -> Result<(), SavedRoomGenerationError> {
    if value.len() != length || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SavedRoomGenerationError::InvalidField {
            field,
            message: format!("must be exactly {length} lowercase hexadecimal characters"),
        });
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(SavedRoomGenerationError::InvalidField {
            field,
            message: "must use lowercase hexadecimal characters".to_string(),
        });
    }
    Ok(())
}

fn validate_digest(field: &'static str, value: &str) -> Result<(), SavedRoomGenerationError> {
    let Some(hex) = value.strip_prefix(DIGEST_PREFIX) else {
        return Err(SavedRoomGenerationError::InvalidField {
            field,
            message: "must use the sha256:<64 lowercase hex> form".to_string(),
        });
    };
    validate_hex(field, hex, 64)
}

fn ensure_digest_matches(
    class: &'static str,
    expected: &str,
    actual: &str,
) -> Result<(), SavedRoomGenerationError> {
    validate_digest(class, expected)?;
    if expected != actual {
        return Err(SavedRoomGenerationError::DigestMismatch {
            class,
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

fn digest_of<T: Serialize>(value: &T) -> Result<String, SavedRoomGenerationError> {
    let value = serde_json::to_value(value)
        .map_err(|error| SavedRoomGenerationError::Encoding(error.to_string()))?;
    let canonical = canonicalize_json(value);
    let encoded = serde_json::to_vec(&canonical)
        .map_err(|error| SavedRoomGenerationError::Encoding(error.to_string()))?;
    Ok(format!("{DIGEST_PREFIX}{:x}", Sha256::digest(encoded)))
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        Value::Object(values) => {
            let mut entries = values.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut canonical = serde_json::Map::new();
            for (key, value) in entries {
                canonical.insert(key, canonicalize_json(value));
            }
            Value::Object(canonical)
        }
        other => other,
    }
}

fn journal_error(operation: &'static str, message: String) -> SavedRoomGenerationError {
    SavedRoomGenerationError::Journal { operation, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonConfig;
    use crate::durable_state::DurableKernelStateStore;
    use crate::session::{
        CanonicalViewport, EnvironmentActionArguments, EnvironmentActionOutcome,
        EnvironmentActionState, EnvironmentMode, EnvironmentTab, RoomEnvironmentSnapshot,
    };

    fn snapshot() -> RoomEnvironmentSnapshot {
        RoomEnvironmentSnapshot {
            session_id: "room-1".to_string(),
            environment_id: "environment-1".to_string(),
            runtime_generation: 2,
            lifecycle: crate::session::EnvironmentLifecycle::Ready,
            health: Vec::new(),
            viewport: CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
            actors: Vec::new(),
            pointers: Vec::new(),
            tabs: vec![EnvironmentTab {
                tab_id: "tab-1".to_string(),
                url: "https://fixture.invalid/room".to_string(),
                title: "Fixture".to_string(),
                document_revision: 3,
                focused: true,
            }],
            focused_tab_id: Some("tab-1".to_string()),
            actions: vec![EnvironmentAction {
                action_id: "action-7".to_string(),
                sequence: 7,
                idempotency_key: Some("idem-7".to_string()),
                actor_id: "user:local".to_string(),
                runtime_generation: 2,
                mode: EnvironmentMode::Browser,
                kind: "observe".to_string(),
                arguments: Some(EnvironmentActionArguments::KeyboardKey { repeat: 1 }),
                targets: vec![InputTarget::BrowserTab("tab-1".to_string())],
                state: EnvironmentActionState::Completed,
                cancellation_requested: false,
                submitted_at_ms: 10,
                started_at_ms: Some(10),
                finished_at_ms: Some(11),
                outcome: Some(EnvironmentActionOutcome::Completed),
            }],
            input_ownership: vec![InputOwnership {
                target: InputTarget::BrowserTab("tab-1".to_string()),
                actor_id: "user:local".to_string(),
            }],
            pending_input_takeovers: Vec::new(),
            event_cursor: 12,
        }
    }

    fn generation(id: &str, previous: Option<SavedRoomGenerationRef>) -> SavedRoomGeneration {
        SavedRoomGeneration::capture(
            id,
            snapshot(),
            vec![SavedPermissionDecision {
                tab_id: "tab-1".to_string(),
                document_revision: 3,
                permission: "notifications".to_string(),
                setting: "granted".to_string(),
            }],
            SavedSourceIdentity {
                commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
                tree: "89abcdef0123456789abcdef0123456789abcdef".to_string(),
            },
            SavedRuntimeImageIdentity {
                digest: format!("{DIGEST_PREFIX}{}", "a".repeat(64)),
            },
            previous,
        )
        .unwrap()
    }

    fn store(label: &str) -> (DurableKernelStateStore, std::path::PathBuf) {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "chariox-saved-room-generation-{label}-{}-{nonce}.db",
            std::process::id()
        ));
        (DurableKernelStateStore::open(path.clone()).unwrap(), path)
    }

    fn cleanup(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn complete_generation_round_trips_and_restores_through_typed_plan() {
        let record = generation("generation-1", None);
        let encoded = serde_json::to_value(&record).unwrap();
        let decoded: SavedRoomGeneration = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, record);
        let plan = decoded.restore_plan().unwrap();
        assert_eq!(plan.generation(), &record);
        assert_eq!(record.action_ledger.next_sequence, 8);
        assert_eq!(record.tabs.focused_tab_id.as_deref(), Some("tab-1"));
    }

    #[test]
    fn every_required_class_omission_is_rejected_before_validation() {
        let record = generation("generation-1", None);
        let required = [
            "room",
            "tabs",
            "action_ledger",
            "permissions",
            "source",
            "runtime_image",
            "class_digests",
            "generation_digest",
        ];
        for field in required {
            let mut value = serde_json::to_value(&record).unwrap();
            value.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<SavedRoomGeneration>(value).is_err(),
                "omitting {field} must fail closed"
            );
        }
    }

    #[test]
    fn corrupted_class_digest_is_rejected_without_a_restore_plan() {
        let mut record = generation("generation-1", None);
        record.class_digests.tab_registry = format!("{DIGEST_PREFIX}{}", "b".repeat(64));
        assert!(matches!(
            record.validate(),
            Err(SavedRoomGenerationError::DigestMismatch {
                class: "tab_registry",
                ..
            })
        ));
        assert!(record.restore_plan().is_err());
    }

    #[test]
    fn cross_generation_mix_is_rejected_before_prepare_and_keeps_n_authoritative() {
        let (store, path) = store("cross-generation");
        let journal = SavedRoomGenerationJournal::new(&store);
        let generation_n = generation("generation-n", None);
        let prepared_n = journal.prepare(generation_n.clone()).unwrap();
        journal.commit(prepared_n).unwrap();

        let mixed = generation(
            "generation-n-plus-one",
            Some(SavedRoomGenerationRef {
                generation_id: "generation-other".to_string(),
                generation_digest: generation("generation-other", None)
                    .generation_digest
                    .clone(),
            }),
        );
        assert!(matches!(
            journal.prepare(mixed),
            Err(SavedRoomGenerationError::CrossGeneration(_))
        ));
        assert_eq!(
            journal
                .latest_committed("room-1", "environment-1")
                .unwrap()
                .unwrap()
                .generation_id,
            "generation-n"
        );
        drop(journal);
        drop(store);
        cleanup(&path);
    }

    #[test]
    fn interrupted_before_commit_keeps_the_last_known_good_generation() {
        let (store, path) = store("interrupted");
        let journal = SavedRoomGenerationJournal::new(&store);
        let generation_n = generation("generation-n", None);
        let prepared_n = journal.prepare(generation_n.clone()).unwrap();
        journal.commit(prepared_n).unwrap();
        let generation_n_plus_one =
            generation("generation-n-plus-one", Some(generation_n.generation_ref()));
        let prepared = journal.prepare(generation_n_plus_one).unwrap();
        assert!(prepared.prepare_event_sequence() > 0);
        drop(journal);
        drop(store);

        let (store, reopened_path) = (DurableKernelStateStore::open(path.clone()).unwrap(), path);
        let journal = SavedRoomGenerationJournal::new(&store);
        assert_eq!(
            journal
                .latest_committed("room-1", "environment-1")
                .unwrap()
                .unwrap()
                .generation_id,
            "generation-n"
        );
        drop(journal);
        drop(store);
        cleanup(&reopened_path);
    }

    #[test]
    fn repeated_rollbacks_preserve_generation_n_as_authoritative() {
        let (store, path) = store("rollback");
        let journal = SavedRoomGenerationJournal::new(&store);
        let generation_n = generation("generation-n", None);
        journal
            .commit(journal.prepare(generation_n.clone()).unwrap())
            .unwrap();
        for attempt in 0..3 {
            let candidate = generation(
                &format!("generation-attempt-{attempt}"),
                Some(generation_n.generation_ref()),
            );
            let prepared = journal.prepare(candidate).unwrap();
            journal
                .rollback(&prepared, format!("deterministic interruption {attempt}"))
                .unwrap();
            let authoritative = journal
                .latest_committed("room-1", "environment-1")
                .unwrap()
                .unwrap();
            assert_eq!(authoritative.generation_id, "generation-n");
            assert_eq!(
                authoritative.generation_digest,
                generation_n.generation_digest
            );
        }
        drop(journal);
        drop(store);
        cleanup(&path);
    }

    #[test]
    fn session_projection_rejects_invalid_generation_without_replacing_known_good_record() {
        let mut sessions = crate::session::SessionService::new(&DaemonConfig::for_tests());
        let session = sessions
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-saved-room-generation",
                "worktree-saved-room-generation",
            ))
            .unwrap();
        sessions
            .create_room_environment(
                session.id(),
                "environment-saved-room-generation",
                CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
            )
            .unwrap();
        let good = sessions
            .capture_saved_room_generation(
                session.id(),
                "generation-1",
                Vec::new(),
                SavedSourceIdentity {
                    commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
                    tree: "89abcdef0123456789abcdef0123456789abcdef".to_string(),
                },
                SavedRuntimeImageIdentity {
                    digest: format!("{DIGEST_PREFIX}{}", "a".repeat(64)),
                },
                None,
            )
            .unwrap();
        sessions
            .restore_saved_room_generations(vec![good.clone()])
            .unwrap();
        let mut invalid = good.clone();
        invalid.generation_digest = format!("{DIGEST_PREFIX}{}", "c".repeat(64));
        assert!(sessions
            .restore_saved_room_generations(vec![invalid])
            .is_err());
        assert_eq!(sessions.saved_room_generation(session.id()), Some(good));
    }
}
