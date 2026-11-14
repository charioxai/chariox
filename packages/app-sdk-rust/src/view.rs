//! A backend that serves its own App view to an agent. The App declares its
//! actions once, in the signed `ui-actions` document the packer puts in the
//! manifest; `View` registers and answers the tools the kernel reads that
//! declaration back through, so the App never names them.
//!
//! The kernel runs a declared action as the backend tool of the same name. The
//! change feed is written by `transact_with_change` in the same transaction as
//! the App's own writes, so a reader never sees a change that did not commit.

use crate::{app_types::AppError, error, get_json, registry, state};
use serde_json::{json, Value};
use std::future::Future;

const ACTIONS_TOOL: &str = "view_actions";
const STATE_TOOL: &str = "view_state";
const CHANGES_TOOL: &str = "view_changes";
const LATEST_KEY: &str = "chariox.view.latest";

/// Changes the feed keeps; an older reader is told to read the state again.
pub const CHANGE_LIMIT: u64 = 128;
/// One change as stored (`seq` and `delta`), so the feed stays within 1 MiB.
pub const CHANGE_BYTES: usize = 8 * 1024;

fn change_key(seq: u64) -> String {
    format!("chariox.view.change.{seq}")
}

pub struct View {
    actions: Value,
    changes: bool,
}

impl View {
    /// `document` is the `{"uiActions": [...]}` text the packer signs.
    pub fn declare(document: &str) -> Result<View, AppError> {
        let mut document: Value = serde_json::from_str(document).map_err(|e| {
            error(
                "INVALID_VIEW",
                format!("the declarations are not JSON: {e}"),
            )
        })?;
        let actions = document["uiActions"].take();
        if !actions.is_array() {
            return Err(error(
                "INVALID_VIEW",
                "the declarations hold no uiActions list",
            ));
        }
        Ok(View {
            actions,
            changes: false,
        })
    }

    /// Offer the revisioned feed that `transact_with_change` writes.
    pub fn with_changes(mut self) -> View {
        self.changes = true;
        self
    }

    /// Seal the view's tools next to the App's own in `Guest::register`.
    pub fn register(&self) {
        registry::tool(ACTIONS_TOOL);
        registry::tool(STATE_TOOL);
        if self.changes {
            registry::tool(CHANGES_TOOL);
        }
    }

    /// Answers a view tool, or `None` when `tool` is one of the App's own. `state` builds the
    /// view's state only when it is asked for.
    pub async fn serve<S, F>(
        &self,
        tool: &str,
        input: &str,
        state: S,
    ) -> Option<Result<String, AppError>>
    where
        S: FnOnce() -> F,
        F: Future<Output = Result<Value, AppError>>,
    {
        match tool {
            ACTIONS_TOOL => Some(Ok(self.actions.to_string())),
            STATE_TOOL => Some(state().await.map(|state| state.to_string())),
            CHANGES_TOOL if self.changes => Some(read_changes(input).await),
            _ => None,
        }
    }
}

/// Commit the App's own transaction together with one change for the feed.
pub async fn transact_with_change(
    mut tx: state::Transaction,
    delta: Value,
) -> Result<state::Committed, AppError> {
    let latest = state::get(LATEST_KEY.to_owned()).await?;
    let (seq, version) = match latest {
        Some(record) => (parse_latest(&record.value)? + 1, Some(record.version)),
        None => (1, None),
    };
    let event = json!({ "seq": seq, "delta": delta }).to_string();
    if event.len() > CHANGE_BYTES {
        return Err(error(
            "CHANGE_TOO_LARGE",
            format!(
                "a change is at most {CHANGE_BYTES} bytes, this one is {}",
                event.len()
            ),
        ));
    }
    tx.checks.push(state::Check {
        key: LATEST_KEY.into(),
        version,
    });
    tx.writes
        .push(state::Write::Put((LATEST_KEY.into(), seq.to_string())));
    tx.writes.push(state::Write::Put((change_key(seq), event)));
    if seq > CHANGE_LIMIT {
        tx.writes
            .push(state::Write::Delete(change_key(seq - CHANGE_LIMIT)));
    }
    state::transact(tx).await
}

fn parse_latest(value: &str) -> Result<u64, AppError> {
    value
        .parse()
        .map_err(|_| error("INVALID_STATE", "the change feed head is not a number"))
}

/// The first sequence number a reader after `since` can still read (up to `latest`), and whether
/// it missed some.
fn feed_window(latest: u64, since: u64) -> (u64, bool) {
    let oldest = latest.saturating_sub(CHANGE_LIMIT - 1).max(1);
    (
        since.saturating_add(1).max(oldest),
        since.saturating_add(1) < oldest,
    )
}

async fn read_changes(input: &str) -> Result<String, AppError> {
    let input: Value =
        serde_json::from_str(input).map_err(|e| error("INVALID_ARGUMENT", e.to_string()))?;
    let since = input["since"].as_u64().unwrap_or(0);
    let latest = match get_json(LATEST_KEY).await? {
        Some((value, _)) => value
            .as_u64()
            .ok_or_else(|| error("INVALID_STATE", "the change feed head is not a number"))?,
        None => 0,
    };
    let (first, dropped) = feed_window(latest, since);
    let mut changes = Vec::new();
    for seq in first..=latest {
        let (event, _) = get_json(&change_key(seq))
            .await?
            .ok_or_else(|| error("INVALID_STATE", format!("change {seq} is missing")))?;
        changes.push(event);
    }
    Ok(json!({ "latest": latest, "dropped": dropped, "changes": changes }).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    const DOCUMENT: &str = r#"{"uiActions":[{"name":"add","effect":"write"}]}"#;

    #[test]
    fn the_declaration_is_the_registration_the_kernel_reads_back() {
        let view = View::declare(DOCUMENT).unwrap();
        let answer = block_on(view.serve(ACTIONS_TOOL, "{}", || async { Ok(json!({})) }));
        let answer: Value = serde_json::from_str(&answer.unwrap().unwrap()).unwrap();
        assert_eq!(
            answer,
            serde_json::from_str::<Value>(DOCUMENT).unwrap()["uiActions"]
        );
    }

    #[test]
    fn state_is_built_only_when_asked_and_an_apps_own_tool_is_not_answered() {
        let view = View::declare(DOCUMENT).unwrap();
        let state = block_on(view.serve(STATE_TOOL, "{}", || async { Ok(json!({"n": 1})) }));
        assert_eq!(state.unwrap().unwrap(), r#"{"n":1}"#);
        let own = block_on(view.serve("add", "{}", || async { panic!("not asked") }));
        assert!(own.is_none());
        let feed = block_on(view.serve(CHANGES_TOOL, "{}", || async { panic!("not asked") }));
        assert!(feed.is_none(), "the feed is offered only by with_changes");
    }

    #[test]
    fn a_document_without_actions_is_refused() {
        assert!(View::declare("{}").is_err());
        assert!(View::declare("not json").is_err());
    }

    #[test]
    fn a_reader_is_told_when_the_feed_moved_past_it() {
        assert_eq!(feed_window(0, 0), (1, false));
        assert_eq!(feed_window(5, 0), (1, false));
        assert_eq!(feed_window(5, 3), (4, false));
        assert_eq!(feed_window(5, 9), (10, false));
        assert_eq!(feed_window(128, 0), (1, false));
        assert_eq!(feed_window(130, 0), (3, true));
        assert_eq!(feed_window(130, 1), (3, true));
        assert_eq!(feed_window(130, 2), (3, false));
    }
}
