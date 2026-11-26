//! Byte budgets are measured after JSON escaping, before the MCP result is encoded.
use super::*;
use serde_json::Value;

pub(super) const LIST_ROWS: u32 = 64;

pub(super) fn page_with_budget<K>(
    rows: impl IntoIterator<Item = (K, Value)>,
    budget: usize,
) -> Result<(Vec<Value>, Option<K>), DaemonError> {
    let mut result = Vec::new();
    let mut used = 2;
    let mut last = None;
    for (key, row) in rows {
        let bytes = row.to_string().len() + 1;
        if bytes + 2 > budget {
            return Err(crate::durable_state::agent_lifecycle::error(
                "List entry exceeds the page budget. Use the entry's detail lookup.",
            ));
        }
        if result.len() == LIST_ROWS as usize || used + bytes > budget {
            return Ok((result, last));
        }
        result.push(row);
        used += bytes;
        last = Some(key);
    }
    Ok((result, None))
}

pub(super) fn cursor_number(args: &Value, field: &str) -> Result<Option<i64>, DaemonError> {
    args.get(field)
        .map(|value| {
            value.as_i64().filter(|number| *number > 0).ok_or_else(|| {
                crate::durable_state::agent_lifecycle::error(format!(
                    "{field} must be the positive cursor returned by the previous page"
                ))
            })
        })
        .transpose()
}

/// Keyset cursors keep their position when a finished wake or an agent is removed.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct CreationKey(pub(super) u64, pub(super) String);

impl CreationKey {
    pub(super) fn cursor(args: &Value, field: &str) -> Result<Option<Self>, DaemonError> {
        args.get(field)
            .map(|value| {
                value
                    .as_str()
                    .filter(|value| value.len() <= 512)
                    .and_then(|value| value.split_once(':'))
                    .and_then(|(time, id)| Some(Self(time.parse().ok()?, id.to_string())))
                    .filter(|key| !key.1.is_empty())
                    .ok_or_else(|| {
                        crate::durable_state::agent_lifecycle::error(format!(
                            "{field} must be the cursor returned by the previous page"
                        ))
                    })
            })
            .transpose()
    }

    pub(super) fn encode(self) -> String {
        format!("{}:{}", self.0, self.1)
    }
}

pub(super) fn include_history(args: &Value) -> Result<bool, DaemonError> {
    args.get("include_history")
        .map(|value| {
            value.as_bool().ok_or_else(|| {
                crate::durable_state::agent_lifecycle::error("include_history must be a boolean")
            })
        })
        .transpose()
        .map(|value| value.unwrap_or(false))
}
