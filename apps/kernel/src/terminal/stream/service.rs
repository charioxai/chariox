use super::json_size::{
    is_coalescible_output_kind, remove_pending_recipient_id, scoped_completion_record,
    scoped_notice_record, scoped_output_record, terminal_output_record_scoped_json_bytes,
};
use super::*;

#[derive(Debug, Clone, Default)]
pub struct TerminalStreamService {
    input_records: Vec<TerminalInputRecord>,
    output_records: BTreeMap<u64, StoredTerminalOutputRecord>,
    pending_output_by_attachment: BTreeMap<(String, String), VecDeque<u64>>,
    next_output_record_id: u64,
    last_output_record_id: Option<u64>,
    notice_records: BTreeMap<u64, RuntimeNoticeRecord>,
    pub(super) pending_notice_by_attachment: BTreeMap<(String, String), VecDeque<u64>>,
    pub(super) pending_broadcast_notice_by_session: BTreeMap<String, VecDeque<u64>>,
    next_notice_record_id: u64,
    pub(super) completion_records: BTreeMap<u64, AssistantMessageCompletionRecord>,
    pub(super) pending_completion_by_attachment: BTreeMap<(String, String), VecDeque<u64>>,
    next_completion_record_id: u64,
    pub(super) workflow_run_update_records: BTreeMap<u64, WorkflowRunUpdateRecord>,
    pub(super) pending_workflow_run_updates_by_attachment:
        BTreeMap<(String, String), VecDeque<u64>>,
    next_workflow_run_update_record_id: u64,
    pending_workflow_run_update_limit_per_attachment: usize,
    pending_output_record_limit_per_attachment: usize,
    output_coalesce_byte_limit: usize,
    output_drain_json_limit: usize,
    trimmed_pending_output_recipients: u64,
    health_store: TerminalStreamHealthStore,
}

fn external_observed_output_content_matches(
    existing: &TerminalOutputRecord,
    next: &TerminalOutputRecord,
) -> bool {
    existing.session_id == next.session_id
        && existing.provider_run_id == next.provider_run_id
        && existing.agent_id == next.agent_id
        && existing.prompt_id == next.prompt_id
        && existing.prompt_origin == next.prompt_origin
        && existing.source_attachment_id == next.source_attachment_id
        && existing.kind == next.kind
        && existing.merge_key == next.merge_key
        && existing.bytes == next.bytes
        && existing.external_observation_metadata == next.external_observation_metadata
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StoredTerminalOutputRecord {
    record: TerminalOutputRecord,
    pending_recipient_count: usize,
}

#[derive(Debug)]
pub(super) struct TerminalOutputBatchFanout {
    pub(super) records: Vec<TerminalOutputRecord>,
    pub(super) changed_keys: BTreeSet<(String, String)>,
}

impl TerminalStreamService {
    pub fn new() -> Self {
        let service = Self {
            pending_workflow_run_update_limit_per_attachment:
                DEFAULT_PENDING_WORKFLOW_RUN_UPDATE_LIMIT_PER_ATTACHMENT,
            pending_output_record_limit_per_attachment:
                DEFAULT_PENDING_OUTPUT_RECORD_LIMIT_PER_ATTACHMENT,
            output_coalesce_byte_limit: DEFAULT_OUTPUT_COALESCE_BYTE_LIMIT,
            output_drain_json_limit: DEFAULT_OUTPUT_DRAIN_JSON_LIMIT,
            ..Self::default()
        };
        service.refresh_health();
        service
    }

    #[cfg(test)]
    pub(super) fn with_pending_output_record_limit_per_attachment(limit: usize) -> Self {
        let service = Self {
            pending_output_record_limit_per_attachment: limit,
            ..Self::new()
        };
        service.refresh_health();
        service
    }

    #[cfg(test)]
    pub(super) fn with_pending_workflow_run_update_limit_per_attachment(limit: usize) -> Self {
        let service = Self {
            pending_workflow_run_update_limit_per_attachment: limit,
            ..Self::new()
        };
        service.refresh_health();
        service
    }

    #[cfg(test)]
    pub(super) fn with_output_coalesce_byte_limit(limit: usize) -> Self {
        let service = Self {
            output_coalesce_byte_limit: limit,
            ..Self::new()
        };
        service.refresh_health();
        service
    }

    #[cfg(test)]
    pub(super) fn with_output_drain_json_limit(limit: usize) -> Self {
        let service = Self {
            output_drain_json_limit: limit,
            ..Self::new()
        };
        service.refresh_health();
        service
    }

    pub fn health_store(&self) -> TerminalStreamHealthStore {
        self.health_store.clone()
    }

    pub fn record_input(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        source_attachment_id: &str,
        bytes: &[u8],
    ) {
        self.input_records.push(TerminalInputRecord {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            source_attachment_id: source_attachment_id.to_string(),
            bytes: bytes.to_vec(),
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        self.fan_out_output_with_prompt_origin(
            session_id,
            provider_run_id,
            agent_id,
            kind,
            merge_key,
            None,
            recipient_attachment_ids,
            bytes,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_output_with_prompt_origin(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        prompt_origin: Option<PromptOrigin>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        self.fan_out_output_with_prompt_metadata(
            session_id,
            provider_run_id,
            agent_id,
            kind,
            merge_key,
            prompt_origin,
            None,
            recipient_attachment_ids,
            bytes,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_output_with_prompt_metadata(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        prompt_origin: Option<PromptOrigin>,
        source_attachment_id: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let record = TerminalOutputRecord {
            record_id: None,
            timestamp_ms: unix_epoch_ms(),
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.map(str::to_string),
            prompt_id: None,
            prompt_origin,
            source_attachment_id,
            kind,
            merge_key,
            pending_recipient_attachment_ids: recipient_attachment_ids.clone(),
            recipient_attachment_ids,
            bytes: bytes.to_vec(),
            external_observation_metadata: None,
        };

        let changed_keys = record
            .pending_recipient_attachment_ids
            .iter()
            .map(|attachment_id| (record.session_id.clone(), attachment_id.clone()))
            .collect::<BTreeSet<_>>();
        if !self.try_coalesce_output_record(&record) {
            self.push_output_record(record.clone());
        }
        self.enforce_pending_output_record_limits_for_keys(changed_keys);
        self.refresh_health();
        record
    }

    pub(super) fn fan_out_outputs(
        &mut self,
        outputs: Vec<TerminalOutputAppend>,
    ) -> TerminalOutputBatchFanout {
        if outputs.is_empty() {
            return TerminalOutputBatchFanout {
                records: Vec::new(),
                changed_keys: BTreeSet::new(),
            };
        }
        let mut records = Vec::with_capacity(outputs.len());
        let mut changed_keys = BTreeSet::new();
        let mut last_recipient_scope: Option<(String, Arc<[String]>)> = None;
        for output in outputs {
            let recipient_attachment_ids = Vec::from(output.recipient_attachment_ids.as_ref());
            let record = TerminalOutputRecord {
                record_id: None,
                timestamp_ms: unix_epoch_ms(),
                session_id: output.session_id,
                provider_run_id: output.provider_run_id,
                agent_id: output.agent_id,
                prompt_id: None,
                prompt_origin: output.prompt_origin,
                source_attachment_id: output.source_attachment_id,
                kind: output.kind,
                merge_key: output.merge_key,
                pending_recipient_attachment_ids: recipient_attachment_ids.clone(),
                recipient_attachment_ids,
                bytes: output.bytes,
                external_observation_metadata: None,
            };
            let same_recipient_scope = last_recipient_scope.as_ref().is_some_and(
                |(session_id, recipient_attachment_ids)| {
                    session_id == &record.session_id
                        && &recipient_attachment_ids[..]
                            == record.recipient_attachment_ids.as_slice()
                },
            );
            if !same_recipient_scope {
                for attachment_id in &record.recipient_attachment_ids {
                    changed_keys.insert((record.session_id.clone(), attachment_id.clone()));
                }
                last_recipient_scope = Some((
                    record.session_id.clone(),
                    output.recipient_attachment_ids.clone(),
                ));
            }

            if !self.try_coalesce_output_record(&record) {
                self.push_output_record(record.clone());
            }
            records.push(record);
        }
        self.enforce_pending_output_record_limits_for_keys(changed_keys.clone());
        self.refresh_health();
        TerminalOutputBatchFanout {
            records,
            changed_keys,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_external_observed_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
        external_observation_metadata: TerminalOutputExternalObservationMetadata,
        source_attachment_id: Option<String>,
    ) -> TerminalOutputRecord {
        let record = TerminalOutputRecord {
            record_id: None,
            timestamp_ms: unix_epoch_ms(),
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.map(str::to_string),
            prompt_id: None,
            prompt_origin: Some(PromptOrigin::External),
            source_attachment_id,
            kind,
            merge_key,
            pending_recipient_attachment_ids: recipient_attachment_ids.clone(),
            recipient_attachment_ids,
            bytes: bytes.to_vec(),
            external_observation_metadata: Some(external_observation_metadata),
        };

        let changed_keys = record
            .pending_recipient_attachment_ids
            .iter()
            .map(|attachment_id| (record.session_id.clone(), attachment_id.clone()))
            .collect::<BTreeSet<_>>();
        if !self.try_replace_external_observed_output_record(&record) {
            self.push_output_record(record.clone());
        }
        self.enforce_pending_output_record_limits_for_keys(changed_keys);
        self.refresh_health();
        record
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_prompt_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        prompt_id: &str,
        prompt_origin: Option<PromptOrigin>,
        source_attachment_id: &str,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        self.fan_out_prompt_output_with_merge_key(
            session_id,
            provider_run_id,
            agent_id,
            prompt_id,
            prompt_origin,
            source_attachment_id,
            None,
            recipient_attachment_ids,
            bytes,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_prompt_output_with_merge_key(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        prompt_id: &str,
        prompt_origin: Option<PromptOrigin>,
        source_attachment_id: &str,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let record = TerminalOutputRecord {
            record_id: None,
            timestamp_ms: unix_epoch_ms(),
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.map(str::to_string),
            prompt_id: Some(prompt_id.to_string()),
            prompt_origin,
            source_attachment_id: Some(source_attachment_id.to_string()),
            kind: TerminalOutputKind::PromptEcho,
            merge_key,
            pending_recipient_attachment_ids: recipient_attachment_ids.clone(),
            recipient_attachment_ids,
            bytes: bytes.to_vec(),
            external_observation_metadata: None,
        };

        let changed_keys = record
            .pending_recipient_attachment_ids
            .iter()
            .map(|attachment_id| (record.session_id.clone(), attachment_id.clone()))
            .collect::<BTreeSet<_>>();
        if !self.try_coalesce_output_record(&record) {
            self.push_output_record(record.clone());
        }
        self.enforce_pending_output_record_limits_for_keys(changed_keys);
        self.refresh_health();
        record
    }

    fn push_output_record(&mut self, record: TerminalOutputRecord) -> u64 {
        let record_id = self.next_output_record_id;
        self.next_output_record_id = self.next_output_record_id.saturating_add(1);
        for attachment_id in &record.pending_recipient_attachment_ids {
            self.pending_output_by_attachment
                .entry((record.session_id.clone(), attachment_id.clone()))
                .or_default()
                .push_back(record_id);
        }
        self.output_records.insert(
            record_id,
            StoredTerminalOutputRecord {
                pending_recipient_count: record.pending_recipient_attachment_ids.len(),
                record,
            },
        );
        self.last_output_record_id = Some(record_id);
        record_id
    }

    fn try_replace_external_observed_output_record(
        &mut self,
        record: &TerminalOutputRecord,
    ) -> bool {
        let Some(merge_key) = record.merge_key.as_deref() else {
            return false;
        };
        let Some(record_id) = self
            .output_records
            .iter()
            .rev()
            .find_map(|(record_id, stored)| {
                (stored.record.session_id == record.session_id
                    && stored.record.agent_id == record.agent_id
                    && stored.record.kind == record.kind
                    && stored.record.merge_key.as_deref() == Some(merge_key)
                    && stored.record.prompt_origin == Some(PromptOrigin::External)
                    && stored.record.external_observation_metadata.is_some())
                .then_some(*record_id)
            })
        else {
            return false;
        };
        let Some(existing_record) = self
            .output_records
            .get(&record_id)
            .map(|stored| stored.record.clone())
        else {
            return false;
        };
        let existing_recipients = existing_record
            .recipient_attachment_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let content_changed = !external_observed_output_content_matches(&existing_record, record);
        let recipients_to_requeue = record
            .recipient_attachment_ids
            .iter()
            .filter(|attachment_id| {
                content_changed || !existing_recipients.contains(*attachment_id)
            })
            .cloned();
        let pending_recipients = existing_record
            .pending_recipient_attachment_ids
            .iter()
            .cloned()
            .chain(recipients_to_requeue)
            .collect::<BTreeSet<_>>();
        let recipient_attachment_ids = existing_record
            .recipient_attachment_ids
            .iter()
            .cloned()
            .chain(record.recipient_attachment_ids.iter().cloned())
            .collect::<BTreeSet<_>>();
        for attachment_id in &pending_recipients {
            let key = (record.session_id.clone(), attachment_id.clone());
            let queue = self.pending_output_by_attachment.entry(key).or_default();
            if !queue.iter().any(|pending_id| *pending_id == record_id) {
                queue.push_back(record_id);
            }
        }
        let mut replacement = if content_changed {
            record.clone()
        } else {
            existing_record
        };
        replacement.recipient_attachment_ids = recipient_attachment_ids.into_iter().collect();
        replacement.pending_recipient_attachment_ids = pending_recipients.iter().cloned().collect();
        let Some(stored) = self.output_records.get_mut(&record_id) else {
            return false;
        };
        stored.record = replacement;
        stored.pending_recipient_count = pending_recipients.len();
        self.last_output_record_id = Some(record_id);
        true
    }

    fn try_coalesce_output_record(&mut self, record: &TerminalOutputRecord) -> bool {
        if self.output_coalesce_byte_limit == 0 || !is_coalescible_output_kind(&record.kind) {
            return false;
        }
        let Some(previous_id) = self.last_output_record_id else {
            return false;
        };
        let Some(previous) = self.output_records.get_mut(&previous_id) else {
            return false;
        };
        if previous.pending_recipient_count != previous.record.recipient_attachment_ids.len()
            || !is_coalescible_output_kind(&previous.record.kind)
            || previous.record.session_id != record.session_id
            || previous.record.provider_run_id != record.provider_run_id
            || previous.record.agent_id != record.agent_id
            || previous.record.prompt_id != record.prompt_id
            || previous.record.prompt_origin != record.prompt_origin
            || previous.record.source_attachment_id != record.source_attachment_id
            || previous.record.external_observation_metadata != record.external_observation_metadata
            || previous.record.kind != record.kind
            || previous.record.merge_key != record.merge_key
            || previous.record.recipient_attachment_ids != record.recipient_attachment_ids
            || previous.record.pending_recipient_attachment_ids
                != record.pending_recipient_attachment_ids
        {
            return false;
        }
        let Some(coalesced_len) = previous.record.bytes.len().checked_add(record.bytes.len())
        else {
            return false;
        };
        if coalesced_len > self.output_coalesce_byte_limit {
            return false;
        }
        previous.record.bytes.extend_from_slice(&record.bytes);
        true
    }

    pub fn record_notice(
        &mut self,
        session_id: &str,
        provider_run_id: Option<&str>,
        agent_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) -> RuntimeNoticeRecord {
        let record = RuntimeNoticeRecord {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.map(str::to_string),
            agent_id: agent_id.map(str::to_string),
            pending_recipient_attachment_ids: recipient_attachment_ids.clone(),
            recipient_attachment_ids,
            message: message.into(),
        };

        self.push_notice_record(record.clone());
        self.refresh_health();
        record
    }

    fn push_notice_record(&mut self, record: RuntimeNoticeRecord) -> u64 {
        let record_id = self.next_notice_record_id;
        self.next_notice_record_id = self.next_notice_record_id.saturating_add(1);
        if record.recipient_attachment_ids.is_empty()
            && record.pending_recipient_attachment_ids.is_empty()
        {
            self.pending_broadcast_notice_by_session
                .entry(record.session_id.clone())
                .or_default()
                .push_back(record_id);
        } else {
            for attachment_id in &record.pending_recipient_attachment_ids {
                self.pending_notice_by_attachment
                    .entry((record.session_id.clone(), attachment_id.clone()))
                    .or_default()
                    .push_back(record_id);
            }
        }
        self.notice_records.insert(record_id, record);
        record_id
    }

    pub fn input_records(&self) -> &[TerminalInputRecord] {
        &self.input_records
    }

    pub fn output_records(&self) -> Vec<TerminalOutputRecord> {
        self.output_records
            .iter()
            .map(|(record_id, stored)| self.output_record_view(*record_id, stored))
            .collect()
    }

    pub fn health_snapshot(&self) -> TerminalStreamHealthSnapshot {
        self.health_store.snapshot()
    }

    fn current_health_snapshot(&self) -> TerminalStreamHealthSnapshot {
        TerminalStreamHealthSnapshot {
            pending_output_records: self.output_records.len(),
            pending_notice_records: self.notice_records.len(),
            pending_completion_records: self.completion_records.len(),
            pending_output_record_limit_per_attachment: self
                .pending_output_record_limit_per_attachment,
            trimmed_pending_output_recipients: self.trimmed_pending_output_recipients,
        }
    }

    fn refresh_health(&self) {
        self.health_store.update(self.current_health_snapshot());
    }

    pub fn drain_output_records(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<TerminalOutputRecord> {
        let mut drained = Vec::new();
        let mut drained_json_bytes: usize = 2;
        let key = (session_id.to_string(), attachment_id.to_string());
        let mut pending_record_ids = self
            .pending_output_by_attachment
            .remove(&key)
            .unwrap_or_default();
        while let Some(record_id) = pending_record_ids.pop_front() {
            let Some((scoped, candidate_json_bytes, final_recipient)) =
                self.output_records.get(&record_id).map(|stored| {
                    let record = &stored.record;
                    let scoped_json_bytes =
                        terminal_output_record_scoped_json_bytes(record, attachment_id);
                    let candidate_json_bytes = if drained.is_empty() {
                        2_usize.saturating_add(scoped_json_bytes)
                    } else {
                        drained_json_bytes
                            .saturating_add(1)
                            .saturating_add(scoped_json_bytes)
                    };
                    (
                        scoped_output_record(record, record_id, attachment_id),
                        candidate_json_bytes,
                        stored.pending_recipient_count <= 1,
                    )
                })
            else {
                continue;
            };
            if !drained.is_empty() && candidate_json_bytes > self.output_drain_json_limit {
                pending_record_ids.push_front(record_id);
                break;
            }
            drained_json_bytes = candidate_json_bytes;
            drained.push(scoped);
            if final_recipient {
                self.remove_output_record(record_id);
            } else {
                self.mark_output_record_drained_for_recipient(record_id, attachment_id);
                self.remove_output_record_if_drained(record_id);
            }
        }
        if !pending_record_ids.is_empty() {
            self.pending_output_by_attachment
                .insert(key, pending_record_ids);
        }

        self.refresh_health();
        drained
    }

    pub(super) fn has_pending_output_records(&self, session_id: &str, attachment_id: &str) -> bool {
        self.pending_output_by_attachment
            .get(&(session_id.to_string(), attachment_id.to_string()))
            .is_some_and(|record_ids| !record_ids.is_empty())
    }

    fn remove_output_record_if_drained(&mut self, record_id: u64) {
        let should_remove = self
            .output_records
            .get(&record_id)
            .is_some_and(|record| record.pending_recipient_count == 0);
        if should_remove {
            self.remove_output_record(record_id);
        }
    }

    fn remove_output_record(&mut self, record_id: u64) {
        self.output_records.remove(&record_id);
        if self.last_output_record_id == Some(record_id) {
            self.last_output_record_id = self.output_records.keys().next_back().copied();
        }
    }

    fn mark_output_record_drained_for_recipient(&mut self, record_id: u64, attachment_id: &str) {
        if let Some(stored) = self.output_records.get_mut(&record_id) {
            remove_pending_recipient_id(
                &mut stored.record.pending_recipient_attachment_ids,
                attachment_id,
            );
            stored.pending_recipient_count = stored.record.pending_recipient_attachment_ids.len();
        }
    }

    fn output_record_view(
        &self,
        record_id: u64,
        stored: &StoredTerminalOutputRecord,
    ) -> TerminalOutputRecord {
        let mut record = stored.record.clone();
        record.record_id = Some(record_id);
        record
    }

    fn enforce_pending_output_record_limits_for_keys(&mut self, keys: BTreeSet<(String, String)>) {
        if keys.is_empty() {
            return;
        }
        if self.pending_output_record_limit_per_attachment == 0 {
            let trimmed = self
                .pending_output_by_attachment
                .iter()
                .filter(|(key, _)| keys.contains(*key))
                .map(|(_, queue)| queue.len() as u64)
                .sum::<u64>();
            self.trimmed_pending_output_recipients = self
                .trimmed_pending_output_recipients
                .saturating_add(trimmed);
            for key in keys {
                if let Some(record_ids) = self.pending_output_by_attachment.remove(&key) {
                    for record_id in record_ids {
                        self.mark_output_record_drained_for_recipient(record_id, &key.1);
                        self.remove_output_record_if_drained(record_id);
                    }
                }
            }
            self.last_output_record_id = self.output_records.keys().next_back().copied();
            return;
        }

        let mut trimmed = 0_u64;
        for key in keys {
            loop {
                let should_trim =
                    self.pending_output_by_attachment
                        .get(&key)
                        .is_some_and(|queue| {
                            queue.len() > self.pending_output_record_limit_per_attachment
                        });
                if !should_trim {
                    break;
                }
                let Some(record_id) = self
                    .pending_output_by_attachment
                    .get_mut(&key)
                    .and_then(|queue| queue.pop_front())
                else {
                    break;
                };
                trimmed = trimmed.saturating_add(1);
                self.mark_output_record_drained_for_recipient(record_id, &key.1);
                self.remove_output_record_if_drained(record_id);
            }
        }
        self.pending_output_by_attachment
            .retain(|_, queue| !queue.is_empty());
        self.trimmed_pending_output_recipients = self
            .trimmed_pending_output_recipients
            .saturating_add(trimmed);
    }

    pub fn notice_records(&self) -> Vec<RuntimeNoticeRecord> {
        self.notice_records.values().cloned().collect()
    }

    pub fn record_assistant_message_completion(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message_id: &str,
        completed_at_ms: u64,
    ) -> AssistantMessageCompletionRecord {
        let record = AssistantMessageCompletionRecord {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.map(str::to_string),
            pending_recipient_attachment_ids: recipient_attachment_ids.clone(),
            recipient_attachment_ids,
            message_id: message_id.to_string(),
            completed_at_ms,
        };

        self.push_completion_record(record.clone());
        self.refresh_health();
        record
    }

    fn push_completion_record(&mut self, record: AssistantMessageCompletionRecord) -> u64 {
        let record_id = self.next_completion_record_id;
        self.next_completion_record_id = self.next_completion_record_id.saturating_add(1);
        for attachment_id in &record.pending_recipient_attachment_ids {
            self.pending_completion_by_attachment
                .entry((record.session_id.clone(), attachment_id.clone()))
                .or_default()
                .push_back(record_id);
        }
        self.completion_records.insert(record_id, record);
        record_id
    }

    pub fn drain_completion_records(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<AssistantMessageCompletionRecord> {
        let mut drained = Vec::new();
        let key = (session_id.to_string(), attachment_id.to_string());
        let completion_ids = self
            .pending_completion_by_attachment
            .remove(&key)
            .unwrap_or_default();
        for record_id in completion_ids {
            let should_drain = self
                .completion_records
                .get(&record_id)
                .is_some_and(|record| {
                    record.session_id == session_id
                        && record
                            .pending_recipient_attachment_ids
                            .iter()
                            .any(|id| id == attachment_id)
                });
            if !should_drain {
                continue;
            }
            if let Some(record) = self.completion_records.get(&record_id) {
                drained.push(scoped_completion_record(record, attachment_id));
            }
            if let Some(record) = self.completion_records.get_mut(&record_id) {
                remove_pending_recipient_id(
                    &mut record.pending_recipient_attachment_ids,
                    attachment_id,
                );
            }
            self.remove_completion_record_if_drained(record_id);
        }

        self.refresh_health();
        drained
    }

    fn remove_completion_record_if_drained(&mut self, record_id: u64) {
        let should_remove = self
            .completion_records
            .get(&record_id)
            .is_some_and(|record| record.pending_recipient_attachment_ids.is_empty());
        if should_remove {
            self.completion_records.remove(&record_id);
        }
    }

    pub fn drain_notice_records(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<RuntimeNoticeRecord> {
        let mut drained = Vec::new();
        let key = (session_id.to_string(), attachment_id.to_string());
        let mut notice_ids = self
            .pending_notice_by_attachment
            .remove(&key)
            .map(|queue| queue.into_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        notice_ids.extend(
            self.pending_broadcast_notice_by_session
                .remove(session_id)
                .unwrap_or_default(),
        );
        notice_ids.sort_unstable();
        notice_ids.dedup();

        for record_id in notice_ids {
            let should_drain = self.notice_records.get(&record_id).is_some_and(|record| {
                record.session_id == session_id
                    && (record.pending_recipient_attachment_ids.is_empty()
                        || record
                            .pending_recipient_attachment_ids
                            .iter()
                            .any(|id| id == attachment_id))
            });
            if !should_drain {
                continue;
            }
            if let Some(record) = self.notice_records.get(&record_id) {
                drained.push(scoped_notice_record(record, attachment_id));
            }
            if let Some(record) = self.notice_records.get_mut(&record_id) {
                remove_pending_recipient_id(
                    &mut record.pending_recipient_attachment_ids,
                    attachment_id,
                );
            }
            self.remove_notice_record_if_drained(record_id);
        }
        self.refresh_health();
        drained
    }

    fn remove_notice_record_if_drained(&mut self, record_id: u64) {
        let should_remove = self.notice_records.get(&record_id).is_some_and(|record| {
            record.recipient_attachment_ids.is_empty()
                || record.pending_recipient_attachment_ids.is_empty()
        });
        if should_remove {
            self.notice_records.remove(&record_id);
        }
    }

    pub fn record_workflow_run_update(
        &mut self,
        session_id: &str,
        recipient_attachment_ids: Vec<String>,
        workflow_run: crate::session::WorkflowRun,
    ) {
        if recipient_attachment_ids.is_empty() {
            return;
        }
        let record_id = self.next_workflow_run_update_record_id;
        self.next_workflow_run_update_record_id =
            self.next_workflow_run_update_record_id.saturating_add(1);
        for attachment_id in &recipient_attachment_ids {
            self.pending_workflow_run_updates_by_attachment
                .entry((session_id.to_string(), attachment_id.clone()))
                .or_default()
                .push_back(record_id);
        }
        let pending_keys = recipient_attachment_ids
            .iter()
            .map(|attachment_id| (session_id.to_string(), attachment_id.clone()))
            .collect();
        self.workflow_run_update_records.insert(
            record_id,
            WorkflowRunUpdateRecord {
                session_id: session_id.to_string(),
                pending_recipient_attachment_ids: recipient_attachment_ids,
                workflow_run,
            },
        );
        self.enforce_pending_workflow_run_update_limits_for_keys(pending_keys);
    }

    fn enforce_pending_workflow_run_update_limits_for_keys(
        &mut self,
        keys: BTreeSet<(String, String)>,
    ) {
        for key in keys {
            while self
                .pending_workflow_run_updates_by_attachment
                .get(&key)
                .is_some_and(|queue| {
                    queue.len() > self.pending_workflow_run_update_limit_per_attachment
                })
            {
                let Some(record_id) = self
                    .pending_workflow_run_updates_by_attachment
                    .get_mut(&key)
                    .and_then(VecDeque::pop_front)
                else {
                    break;
                };
                if let Some(record) = self.workflow_run_update_records.get_mut(&record_id) {
                    remove_pending_recipient_id(
                        &mut record.pending_recipient_attachment_ids,
                        &key.1,
                    );
                }
                if self
                    .workflow_run_update_records
                    .get(&record_id)
                    .is_some_and(|record| record.pending_recipient_attachment_ids.is_empty())
                {
                    self.workflow_run_update_records.remove(&record_id);
                }
            }
        }
        self.pending_workflow_run_updates_by_attachment
            .retain(|_, queue| !queue.is_empty());
    }

    pub fn drain_workflow_run_updates(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<crate::session::WorkflowRun> {
        let mut drained = Vec::new();
        let key = (session_id.to_string(), attachment_id.to_string());
        let record_ids = self
            .pending_workflow_run_updates_by_attachment
            .remove(&key)
            .unwrap_or_default();
        for record_id in record_ids {
            let Some(record) = self.workflow_run_update_records.get_mut(&record_id) else {
                continue;
            };
            drained.push(record.workflow_run.clone());
            remove_pending_recipient_id(
                &mut record.pending_recipient_attachment_ids,
                attachment_id,
            );
            if record.pending_recipient_attachment_ids.is_empty() {
                self.workflow_run_update_records.remove(&record_id);
            }
        }
        drained
    }

    pub fn remove_session(&mut self, session_id: &str) {
        self.input_records
            .retain(|record| record.session_id != session_id);
        let output_record_ids = self
            .output_records
            .iter()
            .filter_map(|(record_id, record)| {
                (record.record.session_id == session_id).then_some(*record_id)
            })
            .collect::<Vec<_>>();
        for record_id in output_record_ids {
            self.output_records.remove(&record_id);
        }
        self.pending_output_by_attachment
            .retain(|(pending_session_id, _), _| pending_session_id != session_id);
        self.last_output_record_id = self.output_records.keys().next_back().copied();
        self.notice_records
            .retain(|_, record| record.session_id != session_id);
        self.pending_notice_by_attachment
            .retain(|(pending_session_id, _), _| pending_session_id != session_id);
        self.pending_broadcast_notice_by_session.remove(session_id);
        self.completion_records
            .retain(|_, record| record.session_id != session_id);
        self.pending_completion_by_attachment
            .retain(|(pending_session_id, _), _| pending_session_id != session_id);
        self.workflow_run_update_records
            .retain(|_, record| record.session_id != session_id);
        self.pending_workflow_run_updates_by_attachment
            .retain(|(pending_session_id, _), _| pending_session_id != session_id);
        self.refresh_health();
    }

    pub fn remove_attachment(&mut self, session_id: &str, attachment_id: &str) -> bool {
        let mut changed = false;
        let key = (session_id.to_string(), attachment_id.to_string());
        if let Some(record_ids) = self.pending_output_by_attachment.remove(&key) {
            changed = true;
            for record_id in record_ids {
                self.mark_output_record_drained_for_recipient(record_id, attachment_id);
                self.remove_output_record_if_drained(record_id);
            }
        }
        if let Some(record_ids) = self.pending_notice_by_attachment.remove(&key) {
            changed = true;
            for record_id in record_ids {
                if let Some(record) = self.notice_records.get_mut(&record_id) {
                    remove_pending_recipient_id(
                        &mut record.pending_recipient_attachment_ids,
                        attachment_id,
                    );
                }
                self.remove_notice_record_if_drained(record_id);
            }
        }
        let broadcast_notice_ids = self
            .pending_broadcast_notice_by_session
            .remove(session_id)
            .unwrap_or_default();
        changed |= !broadcast_notice_ids.is_empty();
        for record_id in broadcast_notice_ids {
            self.notice_records.remove(&record_id);
        }
        if let Some(record_ids) = self.pending_completion_by_attachment.remove(&key) {
            changed = true;
            for record_id in record_ids {
                if let Some(record) = self.completion_records.get_mut(&record_id) {
                    remove_pending_recipient_id(
                        &mut record.pending_recipient_attachment_ids,
                        attachment_id,
                    );
                }
                self.remove_completion_record_if_drained(record_id);
            }
        }
        if let Some(record_ids) = self.pending_workflow_run_updates_by_attachment.remove(&key) {
            changed = true;
            for record_id in record_ids {
                if let Some(record) = self.workflow_run_update_records.get_mut(&record_id) {
                    remove_pending_recipient_id(
                        &mut record.pending_recipient_attachment_ids,
                        attachment_id,
                    );
                }
                if self
                    .workflow_run_update_records
                    .get(&record_id)
                    .is_some_and(|record| record.pending_recipient_attachment_ids.is_empty())
                {
                    self.workflow_run_update_records.remove(&record_id);
                }
            }
        }
        for record in self.notice_records.values_mut() {
            if record.session_id == session_id {
                changed |= remove_pending_recipient_id(
                    &mut record.pending_recipient_attachment_ids,
                    attachment_id,
                );
            }
        }
        for record in self.completion_records.values_mut() {
            if record.session_id == session_id {
                changed |= remove_pending_recipient_id(
                    &mut record.pending_recipient_attachment_ids,
                    attachment_id,
                );
            }
        }
        self.notice_records.retain(|_, record| {
            !record.recipient_attachment_ids.is_empty()
                && !record.pending_recipient_attachment_ids.is_empty()
        });
        self.completion_records
            .retain(|_, record| !record.pending_recipient_attachment_ids.is_empty());
        if changed {
            self.refresh_health();
        }
        changed
    }
}
