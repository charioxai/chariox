//! MP-08 / MP-10 / MP-11: persistent rejection of pre-v2 durable writers.
//! Register on every live writer connection; arm only after all reset phases.
use rusqlite::{functions::FunctionFlags, Connection, OptionalExtension, TransactionBehavior};

const EPOCH_FUNCTION: &str = "chariox_requires_skills_v2_protocol_533";
const EPOCH: i64 = 2;
const TABLES: &[&str] = &[
    "durable_state_events",
    "durable_state_snapshots",
    "durable_state_checkpoint_entities",
    "durable_state_owner_checkpoint_entities",
    "durable_workflow_hot_entities",
    "durable_workflow_runs",
    "durable_workflow_dispatch_intents",
    "durable_state_metadata",
];

pub(super) fn register_writer(connection: &Connection) -> rusqlite::Result<()> {
    connection.create_scalar_function(
        EPOCH_FUNCTION,
        0,
        FunctionFlags::SQLITE_DETERMINISTIC | FunctionFlags::SQLITE_INNOCUOUS,
        |_| Ok(EPOCH),
    )
}

/// The mutable reference, owned startup lease and no runtime admission must be
/// held by the reset coordinator. SQLite DDL + checkpoint commit atomically.
/// This deliberately has no automatic call from the partial reset helpers.
pub(super) fn arm(connection: &mut Connection) -> rusqlite::Result<()> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for key in [
        "authority_reset",
        "filesystem_reset",
        "executable_reference_reset",
    ] {
        let status: Option<String> = tx.query_row(
            "SELECT metadata_value FROM durable_state_metadata WHERE owner_id='skills-v2' AND metadata_key=?1",
            [key], |row| row.get(0)).optional()?;
        if status.as_deref() != Some("complete") {
            return Err(rusqlite::Error::InvalidQuery);
        }
    }
    for table in TABLES {
        for action in ["INSERT", "UPDATE", "DELETE"] {
            // Names are a closed source-defined set, never caller SQL.
            tx.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS skills_v2_writer_{table}_{action}
                BEFORE {action} ON {table} BEGIN
                SELECT CASE WHEN {EPOCH_FUNCTION}() != {EPOCH}
                THEN RAISE(ABORT, 'Skills v2 requires an upgraded kernel writer') END;
                END;"
            ))?;
        }
    }
    tx.execute(
        "INSERT INTO durable_state_metadata(owner_id,metadata_key,metadata_value,updated_at_ms)
        VALUES('skills-v2','writer_epoch','2',0)
        ON CONFLICT(owner_id,metadata_key) DO UPDATE SET metadata_value='2'",
        [],
    )?;
    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable_state::DurableKernelStateStore;
    struct Root(std::path::PathBuf);
    impl Root {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "skillsk-writer-fence-{:032x}",
                rand::random::<u128>()
            ));
            std::fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn mark_reset(connection: &Connection) {
        for key in [
            "authority_reset",
            "filesystem_reset",
            "executable_reference_reset",
        ] {
            connection
                .execute(
                    "INSERT INTO durable_state_metadata VALUES('skills-v2',?1,'complete',0)",
                    [key],
                )
                .unwrap();
        }
    }
    #[test]
    fn skill_reset_writer_fence_requires_every_checkpoint_and_is_idempotent() {
        let root = Root::new();
        let _store = DurableKernelStateStore::open(root.0.join("state.db")).unwrap();
        let mut db = Connection::open(root.0.join("state.db")).unwrap();
        register_writer(&db).unwrap();
        assert!(arm(&mut db).is_err());
        let count:i64=db.query_row("SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name LIKE 'skills_v2_writer_%'",[],|r|r.get(0)).unwrap();
        assert_eq!(count, 0);
        mark_reset(&db);
        arm(&mut db).unwrap();
        arm(&mut db).unwrap();
        let count:i64=db.query_row("SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name LIKE 'skills_v2_writer_%'",[],|r|r.get(0)).unwrap();
        assert_eq!(count, TABLES.len() as i64 * 3);
    }
    #[test]
    fn skill_reset_writer_fence_rejects_old_connections_and_preserves_current_restart() {
        let root = Root::new();
        let path = root.0.join("state.db");
        let store = DurableKernelStateStore::open(path.clone()).unwrap();
        // A real second SQLite connection represents an old writer with no epoch
        // function. It can read history, but must not mutate replay authority.
        let old = Connection::open(&path).unwrap();
        {
            let mut db = Connection::open(&path).unwrap();
            register_writer(&db).unwrap();
            mark_reset(&db);
            arm(&mut db).unwrap();
        }
        store
            .append_event("skills.v2.test", None, serde_json::json!({"ok":true}))
            .unwrap();
        assert!(old.execute("DELETE FROM durable_state_events", []).is_err());
        assert!(old
            .execute(
                "UPDATE durable_state_metadata SET metadata_value='old' WHERE owner_id='skills-v2'",
                []
            )
            .is_err());
        assert_eq!(
            old.query_row("SELECT count(*) FROM durable_state_events", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        drop(old);
        drop(store);
        let restarted = DurableKernelStateStore::open(path).unwrap();
        restarted
            .append_event(
                "skills.v2.test",
                None,
                serde_json::json!({"restarted":true}),
            )
            .unwrap();
        assert_eq!(restarted.latest_event_sequence().unwrap(), 2);
    }
}
