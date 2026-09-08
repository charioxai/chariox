//! Kernel-owned automation configuration on the existing committing transaction.
//! Target strings are data, not proof of workflow ownership. The kernel adapter
//! must hold its workflow guard and validate the authoritative target in this
//! same transaction. This module grants neither workflow nor App permissions.

use super::*;
use rusqlite::{params, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationTarget {
    pub session_id: String,
    pub publication_id: String,
    pub endpoint_id: String,
    pub queue_id: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationStatus {
    Active,
    Paused,
    Broken,
    Disabled,
}
impl AutomationStatus {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "paused" => Ok(Self::Paused),
            "broken" => Ok(Self::Broken),
            "disabled" => Ok(Self::Disabled),
            _ => Err(OutboxError::Corrupt),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Broken => "broken",
            Self::Disabled => "disabled",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationConfiguration {
    pub automation_id: String,
    pub revision: u64,
    pub event_name: String,
    pub event_version: u32,
    pub schema_digest: String,
    pub target: AutomationTarget,
    pub scheduled: bool,
    pub status: AutomationStatus,
}

impl AppOutbox {
    /// Explicit, already authorized kernel configuration. Expected revision zero
    /// means create; all replacements/reactivations increment the current revision.
    /// No cached receipt or old snapshot can silently reactivate this binding.
    pub fn configure_in(
        tx: &Transaction<'_>,
        catalog: &EventCatalog,
        trusted_owner: &str,
        automation_id: &str,
        expected_revision: u64,
        event_name: &str,
        target: &AutomationTarget,
        scheduled: bool,
    ) -> Result<AutomationConfiguration> {
        catalog.require_current(tx, trusted_owner)?;
        for id in [
            trusted_owner,
            automation_id,
            event_name,
            &target.session_id,
            &target.publication_id,
            &target.endpoint_id,
            &target.queue_id,
        ] {
            identifier(id)?;
        }
        let expected = time(expected_revision)?;
        let version = catalog
            .event_version(event_name)
            .ok_or(OutboxError::Schema)?;
        let schema = catalog
            .schema_digest(event_name)
            .ok_or(OutboxError::Schema)?;
        let current: Option<i64> = tx.query_row(
            "SELECT revision FROM app_automations WHERE owner_id=?1 AND installation_id=?2 AND automation_id=?3",
            params![trusted_owner,catalog.installation_id(),automation_id],|row|row.get(0),
        ).optional()?;
        if current != (expected_revision != 0).then_some(expected) {
            return Err(OutboxError::Conflict);
        }
        let next = expected.checked_add(1).ok_or(OutboxError::Limit)?;
        if current.is_none() {
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM app_automations WHERE owner_id=?1 AND installation_id=?2",
                params![trusted_owner, catalog.installation_id()],
                |row| row.get(0),
            )?;
            if count >= MAX_AUTOMATIONS as i64 {
                return Err(OutboxError::Limit);
            }
            tx.execute("INSERT INTO app_automations(owner_id,installation_id,automation_id,revision,event_name,event_version,schema_digest,session_id,publication_id,endpoint_id,queue_id,status,scheduled)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'active',?12)",
                params![trusted_owner,catalog.installation_id(),automation_id,next,event_name,version,schema,target.session_id,target.publication_id,target.endpoint_id,target.queue_id,scheduled])?;
        } else {
            let changed=tx.execute("UPDATE app_automations SET revision=?4,event_name=?5,event_version=?6,schema_digest=?7,session_id=?8,publication_id=?9,endpoint_id=?10,queue_id=?11,status='active',scheduled=?12
                WHERE owner_id=?1 AND installation_id=?2 AND automation_id=?3 AND revision=?13",
                params![trusted_owner,catalog.installation_id(),automation_id,next,event_name,version,schema,target.session_id,target.publication_id,target.endpoint_id,target.queue_id,scheduled,expected])?;
            if changed != 1 {
                return Err(OutboxError::Conflict);
            }
        }
        Self::configuration_in(tx, catalog, trusted_owner, automation_id)
    }

    /// Revoking a binding needs no still-existing workflow target. Active is
    /// intentionally rejected: reactivation must repeat configure_in and the
    /// kernel's current workflow authorization/target checks.
    pub fn deactivate_in(
        tx: &Transaction<'_>,
        catalog: &EventCatalog,
        trusted_owner: &str,
        automation_id: &str,
        expected_revision: u64,
        status: AutomationStatus,
    ) -> Result<AutomationConfiguration> {
        catalog.require_current(tx, trusted_owner)?;
        identifier(automation_id)?;
        if status == AutomationStatus::Active || expected_revision == 0 {
            return Err(OutboxError::Invalid);
        }
        let expected = time(expected_revision)?;
        let next = expected.checked_add(1).ok_or(OutboxError::Limit)?;
        let changed=tx.execute("UPDATE app_automations SET revision=?4,status=?5 WHERE owner_id=?1 AND installation_id=?2 AND automation_id=?3 AND revision=?6",
            params![trusted_owner,catalog.installation_id(),automation_id,next,status.name(),expected])?;
        if changed != 1 {
            return Err(OutboxError::Conflict);
        }
        Self::configuration_in(tx, catalog, trusted_owner, automation_id)
    }
    pub fn configuration_in(
        tx: &Transaction<'_>,
        catalog: &EventCatalog,
        trusted_owner: &str,
        automation_id: &str,
    ) -> Result<AutomationConfiguration> {
        catalog.require_current(tx, trusted_owner)?;
        identifier(automation_id)?;
        let b = admission::load(tx, trusted_owner, catalog.installation_id(), automation_id)?;
        Ok(AutomationConfiguration {
            automation_id: b.id,
            revision: b.revision,
            event_name: b.event_name,
            event_version: b.event_version,
            schema_digest: b.schema_digest,
            target: AutomationTarget {
                session_id: b.session_id,
                publication_id: b.publication_id,
                endpoint_id: b.endpoint_id,
                queue_id: b.queue_id,
            },
            scheduled: b.scheduled,
            status: AutomationStatus::parse(&b.status)?,
        })
    }
    /// Bounded, ordered current-installation query. Retained disabled rows count
    /// against the installation's cap; this method never deletes receipts.
    pub fn configurations_in(
        tx: &Transaction<'_>,
        catalog: &EventCatalog,
        trusted_owner: &str,
    ) -> Result<Vec<AutomationConfiguration>> {
        catalog.require_current(tx, trusted_owner)?;
        let mut statement=tx.prepare("SELECT automation_id FROM app_automations WHERE owner_id=?1 AND installation_id=?2 ORDER BY automation_id LIMIT ?3")?;
        let ids = statement
            .query_map(
                params![
                    trusted_owner,
                    catalog.installation_id(),
                    (MAX_AUTOMATIONS + 1) as i64
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if ids.len() > MAX_AUTOMATIONS {
            return Err(OutboxError::Limit);
        }
        ids.iter()
            .map(|id| Self::configuration_in(tx, catalog, trusted_owner, id))
            .collect()
    }
}
