//! AV3: an App whose backend serves its own actions and state. There is no page,
//! Tab or browser: the kernel reads the registration and the state from the
//! App's reserved tools and runs a declared action as the backend tool of the
//! same name. The signed declarations still decide what may run.

use super::*;
use crate::runtime::state::app_view_runtime::ViewCaller;

/// The backend lists the actions it registers, in the shape of the signed declarations.
const ACTIONS_TOOL: &str = "view_actions";
/// The backend answers with the view's current state.
const STATE_TOOL: &str = "view_state";
/// The backend answers with the changes recorded after `since`, when it keeps a feed.
const CHANGES_TOOL: &str = "view_changes";
/// What one answer of the feed may hold: a reader gets at most this many changes in at most this
/// many bytes, whatever the backend returns.
const FEED_MAX_CHANGES: usize = 128;
const FEED_MAX_BYTES: usize = 1024 * 1024;
/// How often a waiting reader asks the backend again.
const FEED_POLL: std::time::Duration = std::time::Duration::from_millis(250);

fn serves_view(catalog: &chariox_app_runtime::app_catalog::AppCatalog) -> bool {
    [ACTIONS_TOOL, STATE_TOOL]
        .iter()
        .all(|reserved| catalog.tools().any(|tool| tool.local_name == *reserved))
}

impl KernelRuntimeState {
    /// The backend-served view of an App the agent holds, named by its installation id. `None`
    /// when the App is not installed or its backend serves no view: the page path answers then.
    pub(super) async fn backend_view(
        &self,
        agent: &crate::agent::AgentInstance,
        installation: &str,
    ) -> Result<Option<ViewContext>, ViewError> {
        if !agent.has_extension_grant(ExtensionKind::App, installation) {
            return Ok(None);
        }
        let store = self.owned.durable_state_store.clone();
        let (owner, app) = (agent.owner_user_id().to_owned(), installation.to_owned());
        let Ok(Ok(catalog)) =
            tokio::task::spawn_blocking(move || store.active_app_event_catalog(&owner, &app)).await
        else {
            return Ok(None);
        };
        if !serves_view(catalog.app_catalog()) {
            return Ok(None);
        }
        let authority = AppViewAuthority::new(
            self.owned.agent_store.clone(),
            Some(agent),
            installation,
            None,
            Some(self.app_binding_epoch(agent.id(), installation)),
        );
        authority.check()?;
        let changes = catalog
            .app_catalog()
            .tools()
            .any(|tool| tool.local_name == CHANGES_TOOL);
        Ok(Some(ViewContext {
            agent_id: agent.id().to_owned(),
            session: agent.session_id().to_owned(),
            queue_session: agent.session_id().to_owned(),
            target: installation.to_owned(),
            host: ViewHost::Backend { changes },
            binding: AppViewBinding {
                owner: agent.owner_user_id().to_owned(),
                installation: installation.to_owned(),
                generation: catalog.generation(),
                panel: Default::default(),
                logical_tab: None,
            },
            authority,
            declared: catalog.app_catalog().ui_actions().to_vec(),
        }))
    }

    /// What the page would have answered, answered by the backend.
    pub(super) async fn backend_view_call(
        &self,
        context: &ViewContext,
        kind: ViewAgentCallKind,
        invoke: Option<(&str, Value)>,
        changes: Option<(u64, u64)>,
    ) -> Result<Value, ViewError> {
        let agent = self
            .owned
            .agent_store
            .get_agent(&context.agent_id)
            .map_err(|_| app_view_authority::not_granted())?;
        let (state, agent) = (self, &agent);
        let run = |tool: String, input: Value| async move {
            let mut call = state.app_control().views().detached_call();
            tokio::select! {
                result = state.invoke_app_view_tool(ViewCaller::Agent(agent), &context.binding, &tool, input, &mut call) => result,
                () = context.authority.cancelled() => Err(app_view_authority::not_granted()),
            }
        };
        match (kind, invoke) {
            (ViewAgentCallKind::Registrations, _) => run(ACTIONS_TOOL.to_owned(), json!({})).await,
            (ViewAgentCallKind::State, _) => run(STATE_TOOL.to_owned(), json!({})).await,
            (ViewAgentCallKind::Invoke, Some((action, input))) => {
                run(action.to_owned(), input).await
            }
            (ViewAgentCallKind::Invoke, None) => Err(failure(
                "invalid_arguments",
                "an action call names its action",
            )),
            // A preview is shown in the App's own Tab; a backend-served view has none.
            (ViewAgentCallKind::PrepareConfirmation | ViewAgentCallKind::ClearConfirmation, _) => {
                Err(failure(
                    "confirmation_unavailable",
                    "This action needs the person's confirmation in an App Tab: open the App in a Tab first.",
                ))
            }
            (ViewAgentCallKind::Changes, _) => {
                let ViewHost::Backend { changes: true } = context.host else {
                    return Err(failure(
                        "changes_unavailable",
                        "This App's backend keeps no change feed; read its state again with chariox.app_view_state",
                    ));
                };
                let (since, wait_ms) = changes.unwrap_or_default();
                changes_after(since, wait_ms, |since| async move {
                    checked_feed(run(CHANGES_TOOL.to_owned(), json!({ "since": since })).await?)
                })
                .await
            }
        }
    }
}

/// The feed a backend answered, within what a reader may be handed: an object of the head
/// revision, whether the reader missed changes, and at most `FEED_MAX_CHANGES` changes in at most
/// `FEED_MAX_BYTES`, each with a revision above the one before it and not above the head.
fn checked_feed(answer: Value) -> Result<Value, ViewError> {
    let bad = |why: &str| failure("view_error", format!("The backend's change feed {why}"));
    let latest = answer["latest"]
        .as_u64()
        .ok_or_else(|| bad("has no head revision"))?;
    let dropped = answer["dropped"]
        .as_bool()
        .ok_or_else(|| bad("does not say whether changes were lost"))?;
    let changes = answer["changes"]
        .as_array()
        .ok_or_else(|| bad("has no change list"))?;
    if changes.len() > FEED_MAX_CHANGES {
        return Err(bad(&format!(
            "returned more than {FEED_MAX_CHANGES} changes"
        )));
    }
    if answer.to_string().len() > FEED_MAX_BYTES {
        return Err(bad("is larger than 1 MiB"));
    }
    let mut previous = 0;
    for change in changes {
        match change["seq"].as_u64() {
            Some(seq) if seq > previous && seq <= latest => previous = seq,
            _ => return Err(bad("lists revisions out of order")),
        }
    }
    Ok(json!({ "latest": latest, "dropped": dropped, "changes": changes }))
}

/// The changes after `since`; with `wait_ms`, asks again until there is one or the wait is over.
async fn changes_after<F, Fut>(since: u64, wait_ms: u64, mut fetch: F) -> Result<Value, ViewError>
where
    F: FnMut(u64) -> Fut,
    Fut: std::future::Future<Output = Result<Value, ViewError>>,
{
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(wait_ms);
    loop {
        let feed = fetch(since).await?;
        let now = tokio::time::Instant::now();
        if now >= deadline
            || feed["dropped"] == true
            || !feed["changes"].as_array().is_some_and(Vec::is_empty)
        {
            return Ok(feed);
        }
        tokio::time::sleep(FEED_POLL.min(deadline - now)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(seq: u64) -> Value {
        json!({ "seq": seq, "delta": { "n": seq } })
    }

    #[test]
    fn a_feed_is_handed_on_only_within_the_bounds_and_in_order() {
        let ok = json!({ "latest": 130, "dropped": true, "changes": (3..=130).map(change).collect::<Vec<_>>() });
        assert_eq!(checked_feed(ok.clone()).unwrap(), ok);

        let too_many = json!({ "latest": 200, "dropped": false, "changes": (1..=129).map(change).collect::<Vec<_>>() });
        assert!(checked_feed(too_many)
            .unwrap_err()
            .message
            .contains("more than 128"));

        let too_big = json!({ "latest": 1, "dropped": false, "changes": [{ "seq": 1, "delta": "x".repeat(FEED_MAX_BYTES) }] });
        assert!(checked_feed(too_big).unwrap_err().message.contains("1 MiB"));

        for changes in [
            json!([change(2), change(2)]),
            json!([change(3), change(2)]),
            json!([change(6)]),
            json!([{ "delta": 1 }]),
        ] {
            let out_of_order = json!({ "latest": 5, "dropped": false, "changes": changes });
            assert!(checked_feed(out_of_order).is_err(), "{changes}");
        }
        assert!(checked_feed(json!({ "latest": 1, "changes": [] })).is_err());
        assert!(checked_feed(json!([])).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn a_waiting_reader_gets_the_first_change_or_the_empty_answer_at_the_deadline() {
        let asked = std::sync::atomic::AtomicU32::new(0);
        let feed = |since: u64| {
            let asked = asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                // The change lands on the third ask.
                let changes = if asked >= 2 {
                    vec![change(since + 1)]
                } else {
                    vec![]
                };
                Ok(
                    json!({ "latest": since + changes.len() as u64, "dropped": false, "changes": changes }),
                )
            }
        };
        let answer = changes_after(0, 5_000, feed).await.unwrap();
        assert_eq!(answer["changes"], json!([change(1)]));
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 3);

        let quiet = |_| async { Ok(json!({ "latest": 0, "dropped": false, "changes": [] })) };
        let started = tokio::time::Instant::now();
        let answer = changes_after(0, 600, quiet).await.unwrap();
        assert_eq!(answer["changes"], json!([]));
        assert_eq!(started.elapsed(), std::time::Duration::from_millis(600));

        let at_once = changes_after(0, 0, quiet).await.unwrap();
        assert_eq!(at_once["latest"], 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_reader_that_missed_changes_is_told_at_once_and_a_backend_failure_ends_the_wait() {
        let missed = |_| async { Ok(json!({ "latest": 300, "dropped": true, "changes": [] })) };
        let started = tokio::time::Instant::now();
        assert_eq!(
            changes_after(0, 5_000, missed).await.unwrap()["dropped"],
            true
        );
        assert_eq!(started.elapsed(), std::time::Duration::ZERO);

        let failing = |_| async { Err::<Value, _>(failure("view_error", "the backend stopped")) };
        assert!(changes_after(0, 5_000, failing).await.is_err());
    }
}
