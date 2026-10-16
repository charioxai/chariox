//! MP-08 / MP-10 / MP-11: destructive, one-time retirement of skill authority.
//! This is not a legacy skill decoder or a conversion to v2 selections.
use rusqlite::Connection;
use serde_json::Value;
#[path = "skill_reset_files.rs"]
mod files;
pub(crate) use files::{LegacySkillRoot, LegacySkillRootKind};

/// Startup must hold the existing durable owner lease and must not serve turns
/// or catalog mutations until both phases return. Persist the deletion plan
/// before touching files; interrupted deletion does not depend on SKILL.md
/// remaining present. A later v2 install is never rediscovered as legacy state.
pub(crate) fn reset(
    connection: &mut Connection,
    roots: &[LegacySkillRoot],
) -> rusqlite::Result<()> {
    use rusqlite::{params, OptionalExtension, TransactionBehavior};
    connection.pragma_update(None, "synchronous", "FULL")?;
    let plan = {
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<String> = tx.query_row(
            "SELECT metadata_value FROM durable_state_metadata WHERE owner_id='skills-v2' AND metadata_key='filesystem_reset'", [], |r|r.get(0),
        ).optional()?;
        if previous.as_deref() == Some("complete") {
            tx.rollback()?;
            return initialize(connection);
        }
        let plan: Vec<files::OwnedSkillCopy> = match previous {
            Some(encoded) => {
                serde_json::from_str(&encoded).map_err(|_| rusqlite::Error::InvalidQuery)?
            }
            None => files::collect(roots).map_err(|_| rusqlite::Error::InvalidQuery)?,
        };
        let encoded = serde_json::to_string(&plan).map_err(|_| rusqlite::Error::InvalidQuery)?;
        tx.execute("INSERT INTO durable_state_metadata(owner_id,metadata_key,metadata_value,updated_at_ms) VALUES('skills-v2','filesystem_reset',?1,0) ON CONFLICT(owner_id,metadata_key) DO UPDATE SET metadata_value=excluded.metadata_value", [encoded])?;
        tx.commit()?;
        plan
    };
    initialize(connection)?;
    files::remove(&plan).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("UPDATE durable_state_metadata SET metadata_value=?1 WHERE owner_id=?2 AND metadata_key='filesystem_reset'", params!["complete","skills-v2"])?;
    tx.commit()
}

/// Called with the owning startup connection, before the durable writer starts.
/// Existing events and all checkpoint generations are retired in one FULL-sync
/// SQLite transaction, so an older checkpoint cannot resurrect a grant on retry.
/// Filesystem cleanup and startup admission are a separate phase; callers must
/// not mistake this authority checkpoint for completion of the whole reset.
pub(super) fn initialize(connection: &mut Connection) -> rusqlite::Result<()> {
    use rusqlite::{params, OptionalExtension, TransactionBehavior};
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let complete: Option<String> = tx.query_row(
        "SELECT metadata_value FROM durable_state_metadata WHERE owner_id='skills-v2' AND metadata_key='authority_reset'",
        [], |row| row.get(0),
    ).optional()?;
    if complete.as_deref() == Some("complete") {
        return Ok(());
    }
    // Only replayable kernel-owned stores. No App data, credentials, Vault,
    // provider history or transcript database is traversed.
    for table in [
        "durable_state_events",
        "durable_state_snapshots",
        "durable_state_checkpoint_entities",
        "durable_state_owner_checkpoint_entities",
        "durable_workflow_hot_entities",
    ] {
        let mut cursor = i64::MIN;
        loop {
            let query = format!(
                "SELECT rowid,payload_json FROM {table} WHERE rowid>?1 ORDER BY rowid LIMIT 128"
            );
            let rows = tx
                .prepare(&query)?
                .query_map([cursor], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if rows.is_empty() {
                break;
            }
            for (id, encoded) in rows {
                cursor = id;
                // Decode JSON syntax only, never a retired Skill/Grant type.
                let mut value: Value =
                    serde_json::from_str(&encoded).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let before = value.clone();
                retire(&mut value);
                if before != value {
                    let encoded =
                        serde_json::to_string(&value).map_err(|_| rusqlite::Error::InvalidQuery)?;
                    tx.execute(
                        &format!("UPDATE {table} SET payload_json=?1 WHERE rowid=?2"),
                        params![encoded, id],
                    )?;
                }
            }
        }
    }
    tx.execute("INSERT INTO durable_state_metadata(owner_id,metadata_key,metadata_value,updated_at_ms) VALUES('skills-v2','authority_reset','complete',0) ON CONFLICT(owner_id,metadata_key) DO UPDATE SET metadata_value='complete'", [])?;
    tx.commit()
}

fn legacy_grant(value: &Value) -> bool {
    value.get("kind").and_then(Value::as_str) == Some("skill")
}
fn queued_selection(value: &Value) -> bool {
    // Before v2 there is no source identity with which to safely replay these.
    // Cancel explicit selections, including old invocation spelling. Do not
    // reinterpret an unknown $name as a provider-native selection after reset.
    let prompt = value.get("prompt").and_then(Value::as_str).unwrap_or("");
    !crate::skill::selection::explicit_skill_names(prompt).is_empty()
        || prompt.trim_start().starts_with("/skill ")
        || value
            .get("skill_selections")
            .and_then(Value::as_array)
            .is_some_and(|v| !v.is_empty())
        || value
            .get("hidden_system_context")
            .and_then(Value::as_str)
            .is_some_and(|v| {
                v.contains("skill-context-instructions") || v.contains("<chariox_skill")
            })
}
fn retire(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(retire),
        Value::Object(fields) => {
            for (key, field) in fields.iter_mut() {
                match key.as_str() {
                    "extension_grants" | "extensions" => {
                        if let Some(grants) = field.as_array_mut() {
                            grants.retain(|grant| !legacy_grant(grant));
                        }
                    }
                    "skill_grants" | "required_skills" | "skill_selections" => {
                        *field = Value::Array(vec![])
                    }
                    "queued_prompts" | "workflow_queued_prompts" => {
                        if let Some(prompts) = field.as_array_mut() {
                            prompts.retain(|prompt| !queued_selection(prompt));
                            prompts.iter_mut().for_each(retire);
                        }
                    }
                    "active_prompt" if queued_selection(field) => *field = Value::Null,
                    // Typed structural edges only: user JSON, schemas, output and
                    // historical text must never be treated as replay authority.
                    "agent" | "default_agent" | "agents" | "session" | "sessions" | "snapshot"
                    | "requirements" | "workflow" | "workflows" | "nodes" | "definition"
                    | "prompt_runtime" | "active_prompt" | "prompt_state" | "state" => {
                        retire(field)
                    }
                    "prompt_states" => {
                        if let Some(states) = field.as_object_mut() {
                            states.values_mut().for_each(retire);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct TestRoot(std::path::PathBuf);
    impl TestRoot {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "skillsk-reset-journal-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn roots(&self) -> Vec<LegacySkillRoot> {
            vec![LegacySkillRoot {
                path: self.0.join("skills"),
                kind: LegacySkillRootKind::Installed,
            }]
        }
    }
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn skill_reset_resumes_journaled_cleanup_after_partial_deletion() {
        let root = TestRoot::new();
        let path = root.0.join("skills/old");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("SKILL.md"), "old").unwrap();
        std::fs::write(path.join("helper"), "old").unwrap();
        let plan = files::collect(&root.roots()).unwrap();
        let mut db = database();
        db.execute(
            "INSERT INTO durable_state_metadata VALUES('skills-v2','filesystem_reset',?1,0)",
            [serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
        std::fs::remove_file(path.join("SKILL.md")).unwrap();
        reset(&mut db, &root.roots()).unwrap();
        assert!(!path.exists());
        // New v2 data is outside the retired layout and survives the marker.
        let new = root.0.join("skills/packages/skill-v2/digest");
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("SKILL.md"), "new").unwrap();
        reset(&mut db, &root.roots()).unwrap();
        assert!(new.join("SKILL.md").exists());
    }
    #[test]
    #[cfg(unix)]
    fn skill_reset_keeps_pending_checkpoint_when_cleanup_ownership_changes() {
        let root = TestRoot::new();
        let path = root.0.join("skills/old");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("SKILL.md"), "old").unwrap();
        let plan = files::collect(&root.roots()).unwrap();
        let mut db = database();
        db.execute(
            "INSERT INTO durable_state_metadata VALUES('skills-v2','filesystem_reset',?1,0)",
            [serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
        std::fs::rename(&path, root.0.join("original")).unwrap();
        std::os::unix::fs::symlink(root.0.join("original"), &path).unwrap();
        assert!(reset(&mut db, &root.roots()).is_err());
        let marker:String=db.query_row("SELECT metadata_value FROM durable_state_metadata WHERE metadata_key='filesystem_reset'",[],|r|r.get(0)).unwrap();
        assert_ne!(marker, "complete");
        assert!(root.0.join("original/SKILL.md").exists());
    }
    #[test]
    fn skill_reset_removes_only_skill_grants_and_requirements() {
        let mut value = json!({"agents":[{"id":"a", "extension_grants":[
            {"kind":"skill","name":"review"}, {"kind":"mcp","name":"browser"},
            {"kind":"app","name":"app1"}]}], "requirements":{"extensions":[
            {"kind":"skill","name":"review"},{"kind":"script","name":"check"}]}});
        retire(&mut value);
        assert_eq!(
            value["agents"][0]["extension_grants"],
            json!([
            {"kind":"mcp","name":"browser"},{"kind":"app","name":"app1"}])
        );
        assert_eq!(
            value["requirements"]["extensions"],
            json!([{"kind":"script","name":"check"}])
        );
    }
    #[test]
    fn skill_reset_cancels_queued_selection_but_preserves_history() {
        let queued = json!({"id":"q","prompt":"Use $review", "status":"Queued"});
        let ordinary = json!({"id":"o","prompt":"Read the file", "status":"Queued"});
        let mut value = json!({"queued_prompts":[queued.clone(), ordinary.clone()],
            "history":[queued], "active_prompt":{"prompt":"$review","status":"Running"}});
        retire(&mut value);
        assert_eq!(value["queued_prompts"], json!([ordinary]));
        assert!(value["active_prompt"].is_null());
        assert_eq!(value["history"][0]["prompt"], "Use $review");
    }
    #[test]
    fn skill_reset_preserves_currency_shell_variables_and_literal_code() {
        let prompts = [
            "Inspect $HOME/.config",
            "The budget is $42",
            "Discuss `$review` without loading it",
            "```sh\n$review\n```",
            "Read \"$review\" as a quoted example",
            r"The escaped marker is \$review",
            "[attachment]($review)",
            "Explain the /skill command in the old docs",
        ]
        .map(|prompt| json!({"prompt":prompt,"status":"Queued"}));
        let mut value = json!({"queued_prompts":prompts,
            "active_prompt":{"prompt":"Inspect $HOME/.config","status":"Running"}});
        let before = value.clone();
        retire(&mut value);
        assert_eq!(value, before);
        let mut selected = json!({"queued_prompts":[
            {"prompt":"Use $review","status":"Queued"},
            {"prompt":"Read the file","skill_selections":[{"name":"review"}]},
            {"prompt":"/skill review","status":"Queued"}
        ]});
        retire(&mut selected);
        assert_eq!(selected["queued_prompts"], json!([]));
    }
    #[test]
    fn skill_reset_does_not_reinterpret_user_json_or_v2_state() {
        let mut value = json!({"schema":{"properties":{"extension_grants":{"type":"array"}}},
            "history":[{"extension_grants":[{"kind":"skill","name":"old"}]}],
            "skills":{"catalogs":[{"id":"new", "digest":"abc"}]}});
        let original = value.clone();
        retire(&mut value);
        assert_eq!(value, original);
    }
    fn database() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(super::super::DURABLE_STATE_SCHEMA).unwrap();
        c
    }
    #[test]
    fn skill_reset_retires_all_checkpoint_and_replay_copies_atomically() {
        let mut db = database();
        let old = json!({"agent":{"extension_grants":[{"kind":"skill","name":"old"},
            {"kind":"connector","name":"safe"}]}})
        .to_string();
        db.execute(
            "INSERT INTO durable_state_events VALUES (1,'e','agent.extension_granted','a',1,?1)",
            [&old],
        )
        .unwrap();
        db.execute(
            "INSERT INTO durable_state_snapshots VALUES (1,1,1,?1)",
            [&old],
        )
        .unwrap();
        db.execute(
            "INSERT INTO durable_state_checkpoint_entities VALUES ('agents','a',1,?1)",
            [&old],
        )
        .unwrap();
        db.execute(
            "INSERT INTO durable_state_owner_checkpoint_entities VALUES ('home','agents','a',1,?1)",
            [&old],
        )
        .unwrap();
        initialize(&mut db).unwrap();
        for table in [
            "durable_state_events",
            "durable_state_snapshots",
            "durable_state_checkpoint_entities",
            "durable_state_owner_checkpoint_entities",
        ] {
            let value: String = db
                .query_row(&format!("SELECT payload_json FROM {table}"), [], |r| {
                    r.get(0)
                })
                .unwrap();
            let value: Value = serde_json::from_str(&value).unwrap();
            assert_eq!(
                value["agent"]["extension_grants"],
                json!([{"kind":"connector","name":"safe"}]),
                "{table}"
            );
        }
    }
    #[test]
    fn skill_reset_checkpoint_prevents_new_state_deletion_on_restart() {
        let mut db = database();
        initialize(&mut db).unwrap();
        let value = json!({"schema":2,"catalogs":[{"id":"v2"}]}).to_string();
        db.execute(
            "INSERT INTO durable_state_events VALUES (1,'e','skills.v2.catalog','skills-v2',1,?1)",
            [&value],
        )
        .unwrap();
        initialize(&mut db).unwrap();
        let kept: String = db
            .query_row("SELECT payload_json FROM durable_state_events", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(kept, value);
        let status: String = db.query_row("SELECT metadata_value FROM durable_state_metadata WHERE owner_id='skills-v2' AND metadata_key='authority_reset'", [], |r|r.get(0)).unwrap();
        assert_eq!(status, "complete");
    }
    #[test]
    fn skill_reset_corrupt_state_rolls_back_without_checkpoint() {
        let mut db = database();
        let old = json!({"extension_grants":[{"kind":"skill","name":"old"}]}).to_string();
        db.execute(
            "INSERT INTO durable_state_events VALUES (0,'valid','agent.created','a',1,?1)",
            [&old],
        )
        .unwrap();
        db.execute(
            "INSERT INTO durable_state_events VALUES (1,'e','agent.created','a',1,'not-json')",
            [],
        )
        .unwrap();
        assert!(initialize(&mut db).is_err());
        let kept: String = db
            .query_row(
                "SELECT payload_json FROM durable_state_events WHERE event_id='valid'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, old);
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM durable_state_metadata WHERE metadata_key='authority_reset'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}
