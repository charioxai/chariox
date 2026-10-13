//! MP-08/MP-10/MP-11: immutable user-owned invocation provenance, kernel-private.
use super::DurableKernelStateStore;
use crate::error::DaemonError;
use rusqlite::{params, OptionalExtension, Transaction};

#[derive(Debug, Clone)]
pub(crate) struct WorkflowUserInvocation {
    pub(crate) queue_item_id: String,
    pub(crate) user_id: String,
}

fn metadata_key(session_id: &str, queue_item_id: &str) -> String {
    serde_json::to_string(&("workflow.user_invocation", session_id, queue_item_id))
        .expect("string tuple serializes")
}

pub(super) fn write_in(
    tx: &Transaction<'_>,
    owner_id: &str,
    session_id: &str,
    invocation: &WorkflowUserInvocation,
    now: u64,
) -> Result<(), rusqlite::Error> {
    let key = metadata_key(session_id, &invocation.queue_item_id);
    tx.execute(
        "INSERT INTO durable_state_metadata(owner_id,metadata_key,metadata_value,updated_at_ms)
         VALUES(?1,?2,?3,?4) ON CONFLICT(owner_id,metadata_key) DO NOTHING",
        params![owner_id, key, invocation.user_id, now as i64],
    )?;
    let existing: String = tx.query_row(
        "SELECT metadata_value FROM durable_state_metadata WHERE owner_id=?1 AND metadata_key=?2",
        params![owner_id, key],
        |row| row.get(0),
    )?;
    if existing != invocation.user_id {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

impl DurableKernelStateStore {
    pub(crate) fn workflow_invocation_is_owned(
        &self,
        owner_id: &str,
        session_id: &str,
        queue_item_id: &str,
        user_id: &str,
    ) -> Result<bool, DaemonError> {
        let operation = "durable_state.workflow_invocation_is_owned";
        let connection = self.lock_connection(operation)?;
        let creator: Option<String> = connection.query_row(
            "SELECT metadata_value FROM durable_state_metadata WHERE owner_id=?1 AND metadata_key=?2",
            params![owner_id,metadata_key(session_id,queue_item_id)], |row| row.get(0),
        ).optional().map_err(|error| DaemonError::LocalTransport {operation,message:error.to_string()})?;
        Ok(creator.as_deref() == Some(user_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{CollaborationLevel, RuntimeSession, WorkflowRun};

    #[test]
    fn mp11_user_invoker_provenance_is_private_immutable_scoped_and_durable() {
        let root = crate::test_support::TestWorktree::new("workflow-user-provenance");
        let path = root.path().join("state.sqlite");
        let mut room = RuntimeSession::new("room", None, "workspace", "tree", "machine", "home");
        room.add_member("bob", None, CollaborationLevel::Private);
        let make_run = |queue: &str| {
            let mut run = WorkflowRun::new(
                "run",
                "workflow",
                "endpoint",
                "node",
                None,
                None,
                vec![],
                vec![],
            );
            run.set_invocation_context(1, None, Some(queue.into()), 0, None);
            run
        };
        let alice_run = make_run("alice-invocation");
        let bob_run = make_run("bob-invocation");
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            for (queue, user) in [("alice-invocation", "local"), ("bob-invocation", "bob")] {
                store
                    .persist_workflow_runtime_transition_with_invocation(
                        &room,
                        "workflow_prompt_enqueued",
                        Some(WorkflowUserInvocation {
                            queue_item_id: queue.into(),
                            user_id: user.into(),
                        }),
                    )
                    .unwrap();
            }
            assert!(room
                .workflow_run_visible_for_user(&alice_run, "local", Some(&store))
                .unwrap());
            assert!(!room
                .workflow_run_visible_for_user(&alice_run, "bob", Some(&store))
                .unwrap());
            assert!(room
                .workflow_run_visible_for_user(&bob_run, "bob", Some(&store))
                .unwrap());
            assert!(!room
                .workflow_run_visible_for_user(&bob_run, "local", Some(&store))
                .unwrap());
            assert!(!room
                .workflow_run_visible_for_user(&make_run("legacy"), "local", Some(&store))
                .unwrap());
            assert!(!room
                .workflow_run_visible_for_user(
                    &alice_run.clone().with_creator(Some("foreign-agent".into())),
                    "local",
                    Some(&store)
                )
                .unwrap());
            assert!(!store
                .workflow_invocation_is_owned("other-kernel", "room", "alice-invocation", "local")
                .unwrap());
            assert!(!store
                .workflow_invocation_is_owned("home", "other-room", "alice-invocation", "local")
                .unwrap());
        }
        let reopened = DurableKernelStateStore::open(path).unwrap();
        assert!(room
            .workflow_run_visible_for_user(&alice_run, "local", Some(&reopened))
            .unwrap());
        // A conflicting actor aborts the same transaction that admits the queue.
        let mut connection = reopened
            .lock_connection("test invocation rollback")
            .unwrap();
        let tx = connection.transaction().unwrap();
        assert!(write_in(
            &tx,
            "home",
            "room",
            &WorkflowUserInvocation {
                queue_item_id: "alice-invocation".into(),
                user_id: "bob".into(),
            },
            0
        )
        .is_err());
        drop(tx);
        drop(connection);
        assert!(room
            .workflow_run_visible_for_user(&alice_run, "local", Some(&reopened))
            .unwrap());
    }
}
