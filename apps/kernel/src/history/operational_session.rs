use std::collections::{BTreeMap, BTreeSet};

use rusqlite::params;

use crate::error::DaemonError;
use crate::session::prompt_id_number;

use super::session_log::external_provider_observed_merge_key_with_prefix_is_state_signal;
use super::{
    HistoryEvent, OperationalHistoryStore, SessionHistoryEntry, SessionHistoryEntryKind,
    SessionHistoryEntrySource,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalImportHistoryIndex {
    pub arroba_owned_prompts: Vec<String>,
    pub external_entries_by_merge_key: BTreeMap<String, ExternalImportHistoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalImportHistoryEntry {
    pub kind: SessionHistoryEntryKind,
    pub text: String,
    pub external_observation: Option<super::SessionHistoryExternalObservation>,
}

impl OperationalHistoryStore {
    pub fn max_prompt_number(&self) -> Result<u64, DaemonError> {
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT prompt_id
                 FROM history_events
                 WHERE prompt_id IS NOT NULL",
            )
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "prepare operational history prompt id scan",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query([])
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "scan operational history prompt ids",
                message: error.to_string(),
            })?;
        let mut max_prompt_number = 0;
        while let Some(row) = rows
            .next()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "read operational history prompt id",
                message: error.to_string(),
            })?
        {
            let prompt_id =
                row.get::<_, String>(0)
                    .map_err(|error| DaemonError::SessionHistoryFailed {
                        session_id: None,
                        operation: "decode operational history prompt id",
                        message: error.to_string(),
                    })?;
            if let Some(number) = prompt_id_number(&prompt_id) {
                max_prompt_number = max_prompt_number.max(number);
            }
        }
        Ok(max_prompt_number)
    }

    pub fn list_session_history_agent_ids(
        &self,
        session_id: &str,
    ) -> Result<Vec<String>, DaemonError> {
        self.delay_read_if_configured();
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let mut statement = connection
            .prepare(
                "SELECT agent_id
                 FROM history_events
                 WHERE session_id = ?1 AND agent_id IS NOT NULL
                 GROUP BY agent_id
                 ORDER BY MIN(sequence) ASC, agent_id ASC",
            )
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "prepare operational history agent list",
                message: error.to_string(),
            })?;
        let mut rows = statement.query(params![session_id]).map_err(|error| {
            DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "load operational history agent list",
                message: error.to_string(),
            }
        })?;
        let mut agent_ids = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "read operational history agent list",
                message: error.to_string(),
            })?
        {
            agent_ids.push(row.get::<_, String>(0).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "read operational history agent id",
                    message: error.to_string(),
                }
            })?);
        }
        Ok(agent_ids)
    }

    pub fn load_latest_user_prompt_events(
        &self,
        session_id: &str,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<HistoryEvent>, DaemonError> {
        self.delay_read_if_configured();
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let mut statement = connection
            .prepare(
                "SELECT event_json
                 FROM history_events
                 WHERE session_id = ?1 AND agent_id = ?2 AND kind = 'user_prompt'
                 ORDER BY sequence DESC
                 LIMIT ?3",
            )
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "prepare latest user prompt history load",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query(params![session_id, agent_id, limit.max(1) as i64])
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "load latest user prompt history events",
                message: error.to_string(),
            })?;
        let mut events = read_history_events_from_rows(session_id, &mut rows)?;
        events.reverse();
        Ok(events)
    }

    pub fn load_session_events_for_agent_sequence_range(
        &self,
        session_id: &str,
        agent_id: &str,
        sequence_start: u64,
        sequence_end: u64,
    ) -> Result<Vec<HistoryEvent>, DaemonError> {
        self.delay_read_if_configured();
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let mut statement = connection
            .prepare(
                "SELECT event_json
                 FROM history_events
                 WHERE session_id = ?1
                   AND agent_id = ?2
                   AND sequence >= ?3
                   AND sequence <= ?4
                 ORDER BY sequence ASC",
            )
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "prepare operational history range load",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query(params![
                session_id,
                agent_id,
                sequence_start as i64,
                sequence_end as i64
            ])
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "load operational history range events",
                message: error.to_string(),
            })?;
        read_history_events_from_rows(session_id, &mut rows)
    }

    pub fn load_session_events(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> Result<Vec<HistoryEvent>, DaemonError> {
        self.delay_read_if_configured();
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let sql = if agent_id.is_some() {
            "SELECT event_json FROM history_events WHERE session_id = ?1 AND agent_id = ?2 ORDER BY sequence ASC"
        } else {
            "SELECT event_json FROM history_events WHERE session_id = ?1 ORDER BY sequence ASC"
        };
        let mut statement =
            connection
                .prepare(sql)
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "prepare operational history load",
                    message: error.to_string(),
                })?;
        let mut rows = if let Some(agent_id) = agent_id {
            statement.query(params![session_id, agent_id])
        } else {
            statement.query(params![session_id])
        }
        .map_err(|error| DaemonError::SessionHistoryFailed {
            session_id: Some(session_id.to_string()),
            operation: "load operational history events",
            message: error.to_string(),
        })?;
        read_history_events_from_rows(session_id, &mut rows)
    }

    pub fn load_session_history_entries(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> Result<Vec<SessionHistoryEntry>, DaemonError> {
        let events = self.load_session_events(session_id, agent_id)?;
        Ok(events
            .into_iter()
            .filter_map(|event| event.to_session_history_entry())
            .collect())
    }

    pub fn load_external_import_index(
        &self,
        session_id: &str,
        agent_id: &str,
        external_merge_key_prefix: &str,
    ) -> Result<(Vec<String>, BTreeSet<String>), DaemonError> {
        self.delay_read_if_configured();
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let like_pattern = format!("%{external_merge_key_prefix}%");
        let mut statement = connection
            .prepare(
                "SELECT kind,
                        CASE WHEN kind = 'user_prompt' THEN content ELSE NULL END,
                        metadata_text
                 FROM history_events
                 WHERE session_id = ?1
                   AND agent_id = ?2
                   AND (kind = 'user_prompt' OR metadata_text LIKE ?3)
                 ORDER BY sequence ASC",
            )
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "prepare external import history index load",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query(params![session_id, agent_id, like_pattern])
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "load external import history index",
                message: error.to_string(),
            })?;
        let mut arroba_owned_prompts = Vec::new();
        let mut external_merge_keys = BTreeSet::new();
        while let Some(row) = rows
            .next()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "read external import history index",
                message: error.to_string(),
            })?
        {
            let kind =
                row.get::<_, String>(0)
                    .map_err(|error| DaemonError::SessionHistoryFailed {
                        session_id: Some(session_id.to_string()),
                        operation: "decode external import history index kind",
                        message: error.to_string(),
                    })?;
            let content = row.get::<_, Option<String>>(1).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "decode external import history index content",
                    message: error.to_string(),
                }
            })?;
            let metadata_text = row.get::<_, Option<String>>(2).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "decode external import history index metadata",
                    message: error.to_string(),
                }
            })?;
            let metadata_text = metadata_text.unwrap_or_default();
            if kind == "user_prompt"
                && !SessionHistoryEntrySource::metadata_text_contains_external_provider_observed(
                    &metadata_text,
                )
            {
                if let Some(content) = content {
                    arroba_owned_prompts.push(content);
                }
            }
            for line in metadata_text.lines() {
                if line.starts_with(external_merge_key_prefix)
                    && !external_provider_observed_merge_key_with_prefix_is_state_signal(
                        external_merge_key_prefix,
                        line,
                    )
                {
                    external_merge_keys.insert(line.to_string());
                }
            }
        }
        Ok((arroba_owned_prompts, external_merge_keys))
    }

    pub fn load_external_import_history_index(
        &self,
        session_id: &str,
        agent_id: &str,
        external_merge_key_prefix: &str,
    ) -> Result<ExternalImportHistoryIndex, DaemonError> {
        self.delay_read_if_configured();
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let like_pattern = format!("%{external_merge_key_prefix}%");
        let mut statement = connection
            .prepare(
                "SELECT kind, content, metadata_text, event_json
                 FROM history_events
                 WHERE session_id = ?1
                   AND agent_id = ?2
                   AND (kind = 'user_prompt' OR metadata_text LIKE ?3)
                 ORDER BY sequence ASC",
            )
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "prepare external import history entry index load",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query(params![session_id, agent_id, like_pattern])
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "load external import history entry index",
                message: error.to_string(),
            })?;
        let mut arroba_owned_prompts = Vec::new();
        let mut external_entries_by_merge_key = BTreeMap::new();
        while let Some(row) = rows
            .next()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "read external import history entry index",
                message: error.to_string(),
            })?
        {
            let kind =
                row.get::<_, String>(0)
                    .map_err(|error| DaemonError::SessionHistoryFailed {
                        session_id: Some(session_id.to_string()),
                        operation: "decode external import history entry index kind",
                        message: error.to_string(),
                    })?;
            let content = row.get::<_, Option<String>>(1).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "decode external import history entry index content",
                    message: error.to_string(),
                }
            })?;
            let metadata_text = row.get::<_, Option<String>>(2).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "decode external import history entry index metadata",
                    message: error.to_string(),
                }
            })?;
            let event_json =
                row.get::<_, String>(3)
                    .map_err(|error| DaemonError::SessionHistoryFailed {
                        session_id: Some(session_id.to_string()),
                        operation: "decode external import history entry index event json",
                        message: error.to_string(),
                    })?;
            let metadata_text = metadata_text.unwrap_or_default();
            let external_observation = serde_json::from_str::<HistoryEvent>(&event_json)
                .ok()
                .and_then(|event| event.to_session_history_entry())
                .and_then(|entry| entry.external_observation);
            let is_external_observed =
                SessionHistoryEntrySource::metadata_text_contains_external_provider_observed(
                    &metadata_text,
                );
            if kind == "user_prompt" && !is_external_observed {
                if let Some(content) = content.clone() {
                    arroba_owned_prompts.push(content);
                }
            }
            let Some(kind) = session_history_kind_from_key(&kind) else {
                continue;
            };
            let Some(content) = content else {
                continue;
            };
            for line in metadata_text.lines() {
                if line.starts_with(external_merge_key_prefix)
                    && !external_provider_observed_merge_key_with_prefix_is_state_signal(
                        external_merge_key_prefix,
                        line,
                    )
                {
                    external_entries_by_merge_key.insert(
                        line.to_string(),
                        ExternalImportHistoryEntry {
                            kind,
                            text: content.clone(),
                            external_observation: external_observation.clone(),
                        },
                    );
                }
            }
        }
        Ok(ExternalImportHistoryIndex {
            arroba_owned_prompts,
            external_entries_by_merge_key,
        })
    }

    pub fn load_arroba_owned_prompt_texts(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<String>, DaemonError> {
        self.delay_read_if_configured();
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let mut statement = connection
            .prepare(
                "SELECT content, metadata_text
                 FROM history_events
                 WHERE session_id = ?1
                   AND agent_id = ?2
                   AND kind = 'user_prompt'
                 ORDER BY sequence ASC",
            )
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "prepare arroba-owned prompt history load",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query(params![session_id, agent_id])
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "load arroba-owned prompt history",
                message: error.to_string(),
            })?;
        let mut prompts = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "read arroba-owned prompt history",
                message: error.to_string(),
            })?
        {
            let content = row.get::<_, Option<String>>(0).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "decode arroba-owned prompt content",
                    message: error.to_string(),
                }
            })?;
            let metadata_text = row.get::<_, Option<String>>(1).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "decode arroba-owned prompt metadata",
                    message: error.to_string(),
                }
            })?;
            if SessionHistoryEntrySource::metadata_text_contains_external_provider_observed(
                &metadata_text.unwrap_or_default(),
            ) {
                continue;
            }
            if let Some(content) = content {
                prompts.push(content);
            }
        }
        Ok(prompts)
    }

    pub fn has_session_events(&self, session_id: &str) -> Result<bool, DaemonError> {
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM history_events WHERE session_id = ?1 LIMIT 1)",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value != 0)
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "check operational session history",
                message: error.to_string(),
            })
    }

    pub fn legacy_fallback_disabled(&self, session_id: &str) -> Result<bool, DaemonError> {
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        connection
            .query_row(
                "SELECT legacy_fallback_disabled_at_ms IS NOT NULL
                 FROM history_session_markers
                 WHERE session_id = ?1",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value != 0)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(false),
                error => Err(DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "check legacy history fallback marker",
                    message: error.to_string(),
                }),
            })
    }
}

fn session_history_kind_from_key(kind: &str) -> Option<SessionHistoryEntryKind> {
    match kind {
        "user_prompt" => Some(SessionHistoryEntryKind::UserPrompt),
        "provider_output" => Some(SessionHistoryEntryKind::ProviderOutput),
        "provider_reasoning" => Some(SessionHistoryEntryKind::ProviderReasoning),
        "provider_tool" => Some(SessionHistoryEntryKind::ProviderTool),
        "provider_error" => Some(SessionHistoryEntryKind::ProviderError),
        "provider_status" => Some(SessionHistoryEntryKind::ProviderStatus),
        "notice" => Some(SessionHistoryEntryKind::Notice),
        _ => None,
    }
}

fn read_history_events_from_rows(
    session_id: &str,
    rows: &mut rusqlite::Rows<'_>,
) -> Result<Vec<HistoryEvent>, DaemonError> {
    let mut events = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| DaemonError::SessionHistoryFailed {
            session_id: Some(session_id.to_string()),
            operation: "read operational history event",
            message: error.to_string(),
        })?
    {
        let event_json =
            row.get::<_, String>(0)
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "read operational history event",
                    message: error.to_string(),
                })?;
        let event = serde_json::from_str::<HistoryEvent>(&event_json).map_err(|error| {
            DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "decode operational history event",
                message: error.to_string(),
            }
        })?;
        events.push(event);
    }
    Ok(events)
}
