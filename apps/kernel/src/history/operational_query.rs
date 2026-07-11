use crate::error::DaemonError;

use super::{HistoryEvent, HistoryEventQuery, OperationalHistoryStore};

impl OperationalHistoryStore {
    pub fn query_events(&self, query: HistoryEventQuery) -> Result<Vec<HistoryEvent>, DaemonError> {
        self.delay_read_if_configured();
        let connection = self.lock_read_connection(query.session_id.as_deref())?;
        let mut sql = String::from("SELECT event_json FROM history_events WHERE 1 = 1");
        let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        push_optional_filter(&mut sql, &mut values, "session_id", query.session_id);
        push_optional_filter(&mut sql, &mut values, "agent_id", query.agent_id);
        push_optional_filter(&mut sql, &mut values, "provider", query.provider);
        push_optional_filter(&mut sql, &mut values, "model", query.model);
        push_optional_filter(&mut sql, &mut values, "workflow_id", query.workflow_id);
        push_optional_filter(&mut sql, &mut values, "machine_id", query.machine_id);
        push_optional_filter(&mut sql, &mut values, "repo_root", query.repo_root);
        push_optional_filter(&mut sql, &mut values, "worktree_path", query.worktree_path);
        push_optional_filter(&mut sql, &mut values, "kind", query.kind);
        if let Some(after_sequence) = query.after_sequence {
            sql.push_str(" AND sequence > ?");
            values.push(Box::new(after_sequence as i64));
        }
        if let Some(before_sequence) = query.before_sequence {
            sql.push_str(" AND sequence < ?");
            values.push(Box::new(before_sequence as i64));
        }
        if let Some(text) = query.text.filter(|value| !value.trim().is_empty()) {
            sql.push_str(" AND (content LIKE ? ESCAPE '\\' OR metadata_text LIKE ? ESCAPE '\\')");
            let pattern = format!("%{}%", escape_sql_like(&text));
            values.push(Box::new(pattern.clone()));
            values.push(Box::new(pattern));
        }
        let reverse_before_page = query.before_sequence.is_some() && query.after_sequence.is_none();
        if reverse_before_page {
            sql.push_str(" ORDER BY sequence DESC LIMIT ?");
        } else {
            sql.push_str(" ORDER BY sequence ASC LIMIT ?");
        }
        values.push(Box::new(query.limit.unwrap_or(100).clamp(1, 500) as i64));
        let params = rusqlite::params_from_iter(values.iter().map(|value| value.as_ref()));
        let mut statement =
            connection
                .prepare(&sql)
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "prepare operational history query",
                    message: error.to_string(),
                })?;
        let mut rows =
            statement
                .query(params)
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "query operational history",
                    message: error.to_string(),
                })?;
        let mut events = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "read operational history query row",
                message: error.to_string(),
            })?
        {
            let event_json =
                row.get::<_, String>(0)
                    .map_err(|error| DaemonError::SessionHistoryFailed {
                        session_id: None,
                        operation: "read operational history query event",
                        message: error.to_string(),
                    })?;
            let event = serde_json::from_str::<HistoryEvent>(&event_json).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "decode operational history query event",
                    message: error.to_string(),
                }
            })?;
            events.push(event);
        }
        if reverse_before_page {
            events.reverse();
        }
        Ok(events)
    }
}

fn push_optional_filter(
    sql: &mut String,
    values: &mut Vec<Box<dyn rusqlite::ToSql>>,
    column: &'static str,
    value: Option<String>,
) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        sql.push_str(" AND ");
        sql.push_str(column);
        sql.push_str(" = ?");
        values.push(Box::new(value));
    }
}

fn escape_sql_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
