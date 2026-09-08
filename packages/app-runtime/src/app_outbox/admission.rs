use super::*;
use chariox_app_package::Limits;
use rusqlite::OptionalExtension;
use std::collections::BTreeMap;

struct Event {
    version: u32,
    schema_digest: String,
    validator: jsonschema::JSONSchema,
}

/// Signed event schemas tied to the exact admitted installation catalog.
/// Both the version and payload schema come from the publisher-signed event
/// declaration. An emitted version cannot authenticate its own schema.
pub struct EventCatalog {
    catalog: Arc<AppCatalog>,
    events: BTreeMap<String, Event>,
}
impl EventCatalog {
    pub fn compile(package: &VerifiedPackage<'_>, catalog: Arc<AppCatalog>) -> Result<Self> {
        if package.package_digest() != catalog.package_digest() {
            return Err(OutboxError::Conflict);
        }
        let mut events = BTreeMap::new();
        for declaration in &package.declarations().events {
            let schema = serde_json_canonicalizer::to_vec(&declaration.payload_schema)
                .map_err(|_| OutboxError::Schema)?;
            let validator = chariox_app_package::compile_schema(
                &declaration.payload_schema,
                true,
                &Limits::default(),
            )
            .map_err(|_| OutboxError::Schema)?;
            events.insert(
                declaration.name.clone(),
                Event {
                    version: declaration.schema_version,
                    schema_digest: digest(&schema),
                    validator,
                },
            );
        }
        Ok(Self { catalog, events })
    }
    pub fn installation_id(&self) -> &str {
        self.catalog.installation_id()
    }
    pub fn generation(&self) -> u64 {
        self.catalog.generation()
    }
    pub fn schema_digest(&self, name: &str) -> Option<&str> {
        self.events
            .get(name)
            .map(|event| event.schema_digest.as_str())
    }
    pub fn event_version(&self, name: &str) -> Option<u32> {
        self.events.get(name).map(|event| event.version)
    }
    pub(super) fn require_current(&self, tx: &Transaction<'_>, owner: &str) -> Result<()> {
        self.catalog.require_current(tx, owner)?;
        Ok(())
    }
}

/// Opaque snapshot loaded from the existing kernel database. No public
/// constructor accepts App-supplied targets, versions, owners or permissions.
pub struct VerifiedAutomation {
    pub(super) catalog: Arc<EventCatalog>,
    pub(super) owner: String,
    pub(super) binding: Binding,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Binding {
    pub id: String,
    pub revision: u64,
    pub event_name: String,
    pub event_version: u32,
    pub schema_digest: String,
    pub session_id: String,
    pub publication_id: String,
    pub endpoint_id: String,
    pub queue_id: String,
    pub status: String,
    pub scheduled: bool,
}
impl VerifiedAutomation {
    pub fn load_in(
        tx: &Transaction<'_>,
        catalog: Arc<EventCatalog>,
        trusted_owner: &str,
        automation_id: &str,
        expected_revision: u64,
    ) -> Result<Self> {
        identifier(automation_id)?;
        identifier(trusted_owner)?;
        if expected_revision == 0 || expected_revision > i64::MAX as u64 {
            return Err(OutboxError::Invalid);
        }
        catalog.require_current(tx, trusted_owner)?;
        let binding = load(tx, trusted_owner, catalog.installation_id(), automation_id)?;
        if binding.revision != expected_revision {
            return Err(OutboxError::Conflict);
        }
        let result = Self {
            catalog,
            owner: trusted_owner.into(),
            binding,
        };
        result.check_schema()?;
        Ok(result)
    }
    pub fn id(&self) -> &str {
        &self.binding.id
    }
    pub fn revision(&self) -> u64 {
        self.binding.revision
    }
    pub fn event_version(&self) -> u32 {
        self.binding.event_version
    }
    pub fn installation_id(&self) -> &str {
        self.catalog.installation_id()
    }

    pub(super) fn require_current(&self, tx: &Transaction<'_>) -> Result<()> {
        self.catalog.require_current(tx, &self.owner)?;
        if load(tx, &self.owner, self.installation_id(), self.id())? != self.binding {
            return Err(OutboxError::Conflict);
        }
        self.check_schema()
    }
    fn check_schema(&self) -> Result<()> {
        if self.binding.status != "active" {
            return Err(OutboxError::Inactive);
        }
        if self.catalog.schema_digest(&self.binding.event_name)
            != Some(self.binding.schema_digest.as_str())
            || self.catalog.event_version(&self.binding.event_name)
                != Some(self.binding.event_version)
        {
            return Err(OutboxError::Schema);
        }
        Ok(())
    }
    pub(super) fn encode_payload(&self, value: &Value) -> Result<String> {
        let mut nodes = 16_384;
        let mut bytes = MAX_PAYLOAD_BYTES;
        bound(value, 0, &mut nodes, &mut bytes)?;
        let event = self
            .catalog
            .events
            .get(&self.binding.event_name)
            .ok_or(OutboxError::Schema)?;
        if !event.validator.is_valid(value) {
            return Err(OutboxError::Schema);
        }
        let encoded =
            serde_json_canonicalizer::to_string(value).map_err(|_| OutboxError::Invalid)?;
        if encoded.len() > MAX_PAYLOAD_BYTES {
            return Err(OutboxError::Limit);
        }
        Ok(encoded)
    }
}

fn load(connection: &Connection, owner: &str, installation: &str, id: &str) -> Result<Binding> {
    let binding = connection.query_row(
        "SELECT automation_id,revision,event_name,event_version,schema_digest,session_id,publication_id,endpoint_id,queue_id,status,scheduled
         FROM app_automations WHERE owner_id=?1 AND installation_id=?2 AND automation_id=?3",
        rusqlite::params![owner,installation,id],
        |row| Ok(Binding {
            id:row.get(0)?, revision:row.get::<_,i64>(1)?.try_into().map_err(|_|rusqlite::Error::InvalidQuery)?,
            event_name:row.get(2)?, event_version:row.get::<_,i64>(3)?.try_into().map_err(|_|rusqlite::Error::InvalidQuery)?,
            schema_digest:row.get(4)?,session_id:row.get(5)?,publication_id:row.get(6)?,endpoint_id:row.get(7)?,queue_id:row.get(8)?,status:row.get(9)?,
            scheduled:match row.get::<_,i64>(10)? {0=>false,1=>true,_=>return Err(rusqlite::Error::InvalidQuery)},
        }),
    ).optional()?.ok_or(OutboxError::NotFound)?;
    if binding.revision == 0 || binding.event_version == 0 {
        return Err(OutboxError::Corrupt);
    }
    for id in [
        &binding.id,
        &binding.event_name,
        &binding.session_id,
        &binding.publication_id,
        &binding.endpoint_id,
        &binding.queue_id,
    ] {
        identifier(id).map_err(|_| OutboxError::Corrupt)?;
    }
    Ok(binding)
}

fn bound(value: &Value, depth: usize, nodes: &mut usize, bytes: &mut usize) -> Result<()> {
    *nodes = nodes.checked_sub(1).ok_or(OutboxError::Limit)?;
    *bytes = bytes.checked_sub(1).ok_or(OutboxError::Limit)?;
    match value {
        Value::Object(values) => {
            if depth >= 32 {
                return Err(OutboxError::Limit);
            }
            for (key, value) in values {
                *bytes = bytes.checked_sub(key.len()).ok_or(OutboxError::Limit)?;
                bound(value, depth + 1, nodes, bytes)?;
            }
        }
        Value::Array(values) => {
            if depth >= 32 {
                return Err(OutboxError::Limit);
            }
            for value in values {
                bound(value, depth + 1, nodes, bytes)?;
            }
        }
        Value::String(value) => {
            *bytes = bytes.checked_sub(value.len()).ok_or(OutboxError::Limit)?
        }
        Value::Number(number) => {
            let value = number.as_f64().ok_or(OutboxError::Invalid)?;
            if !value.is_finite() || (value.fract() == 0.0 && value.abs() > 9_007_199_254_740_991.0)
            {
                return Err(OutboxError::Invalid);
            }
        }
        _ => {}
    }
    Ok(())
}
