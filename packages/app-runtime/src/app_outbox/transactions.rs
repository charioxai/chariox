use super::*;
use std::collections::{BTreeMap, BTreeSet};

impl AppOutbox {
    /// Bounded shape checking before handing untrusted input to the writer.
    /// This authenticates neither schemas nor automation admission.
    pub fn validate_occurrences(occurrences: &[Occurrence]) -> Result<()> {
        if occurrences.is_empty() || occurrences.len() > MAX_BATCH {
            return Err(OutboxError::Limit);
        }
        let mut identities = BTreeSet::new();
        let mut bytes = 0usize;
        for occurrence in occurrences {
            identifier(&occurrence.automation_id)?;
            identity::validate(&occurrence.occurrence_id, occurrence.occurred_at_ms)?;
            if occurrence.event_version == 0 || occurrence.occurred_at_ms > MAX_SAFE_TIMESTAMP {
                return Err(OutboxError::Invalid);
            }
            if let Some(revision) = &occurrence.schedule_revision {
                identifier(revision)?;
            }
            if !identities.insert((
                &occurrence.automation_id,
                &occurrence.occurrence_id,
                &occurrence.schedule_revision,
            )) {
                return Err(OutboxError::Invalid);
            }
            let payload = admission::encode_bounded_payload(&occurrence.payload)?;
            let invocation = occurrence.invocation.encode()?;
            bytes = bytes
                .checked_add(payload.len())
                .and_then(|size| size.checked_add(invocation.len()))
                .ok_or(OutboxError::Limit)?;
            if bytes > MAX_BATCH_BYTES {
                return Err(OutboxError::Limit);
            }
        }
        Ok(())
    }

    /// Resolves each current automation on the committing writer connection.
    /// The whole batch shares one savepoint and the global 16-item/512-KiB cap.
    /// Returned receipts are provisional until the outer writer commits.
    pub fn apply_current_in(
        tx: &mut Transaction<'_>,
        catalog: Arc<EventCatalog>,
        trusted_owner: &str,
        occurrences: &[Occurrence],
        now: u64,
    ) -> Result<Vec<Receipt>> {
        Self::validate_occurrences(occurrences)?;
        time(now)?;
        catalog.require_current(tx, trusted_owner)?;
        let mut automations = BTreeMap::new();
        for occurrence in occurrences {
            if !automations.contains_key(&occurrence.automation_id) {
                let current = admission::load(
                    tx,
                    trusted_owner,
                    catalog.installation_id(),
                    &occurrence.automation_id,
                )?;
                let automation = VerifiedAutomation::load_in(
                    tx,
                    catalog.clone(),
                    trusted_owner,
                    &occurrence.automation_id,
                    current.revision,
                )?;
                automations.insert(occurrence.automation_id.clone(), automation);
            }
        }
        let prepared = occurrences
            .iter()
            .map(|occurrence| {
                let automation = automations
                    .get(&occurrence.automation_id)
                    .ok_or(OutboxError::Corrupt)?;
                let (_, payload, invocation) =
                    prepare(automation, std::slice::from_ref(occurrence), now)?.remove(0);
                Ok((automation, occurrence, payload, invocation))
            })
            .collect::<Result<Vec<_>>>()?;
        let savepoint = tx.savepoint()?;
        let receipts = prepared
            .iter()
            .map(|(automation, occurrence, payload, invocation)| {
                store::accept(&savepoint, automation, occurrence, payload, invocation, now)
            })
            .collect::<Result<Vec<_>>>()?;
        savepoint.commit()?;
        Ok(receipts)
    }

    /// Acknowledges only a previously kernel-classified retry that is eligible
    /// now. This never resets attempts, changes backoff, or repeats external work.
    pub fn reconcile_retry_in(
        tx: &Transaction<'_>,
        catalog: &EventCatalog,
        trusted_owner: &str,
        receipt_id: &str,
        now: u64,
    ) -> Result<Receipt> {
        let receipt = Self::status_in(tx, catalog, trusted_owner, receipt_id)?;
        time(now)?;
        let binding = admission::load(
            tx,
            trusted_owner,
            catalog.installation_id(),
            &receipt.automation_id,
        )?;
        if binding.status != "active" {
            return Err(OutboxError::Inactive);
        }
        if catalog.schema_digest(&binding.event_name) != Some(binding.schema_digest.as_str())
            || catalog.event_version(&binding.event_name) != Some(binding.event_version)
        {
            return Err(OutboxError::Schema);
        }
        if receipt.state != ReceiptState::Retryable
            || receipt.automation_revision != binding.revision
            || receipt.event_version != binding.event_version
            || receipt.attempts >= MAX_ATTEMPTS
            || now < receipt.accepted_at_ms
            || now < receipt.next_attempt_at_ms
            || now >= receipt.expires_at_ms
            || receipt.payload.is_none()
            || receipt.invocation.is_none()
        {
            return Err(OutboxError::Conflict);
        }
        Ok(receipt)
    }
}
