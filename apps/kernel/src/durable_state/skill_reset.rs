//! MP-08 / MP-10 / MP-11: destructive, one-time retirement of skill authority.
//! This is not a legacy skill decoder or a conversion to v2 selections.
use rusqlite::Connection;
use serde_json::Value;
#[path = "skill_reset_executable.rs"]
mod executable;
#[path = "skill_reset_files.rs"]
mod files;
#[path = "skill_reset_publications.rs"]
mod publications;
pub(crate) use executable::{
    collect_names as collect_retired_skill_names, load_names as retired_skill_names,
    retire_definition as retire_workflow_skill_references,
};
pub(crate) use files::{LegacySkillRoot, LegacySkillRootKind};

const REPLAY_TABLES: &[(&str, &str)] = &[
    ("durable_state_events", "kind"),
    ("durable_state_snapshots", "''"),
    ("durable_state_checkpoint_entities", "entity_kind"),
    ("durable_state_owner_checkpoint_entities", "entity_kind"),
    ("durable_workflow_hot_entities", "entity_kind"),
];

/// Startup must hold the existing durable owner lease and must not serve turns
/// or catalog mutations until both phases return. Persist the deletion plan
/// before touching files; interrupted deletion does not depend on SKILL.md
/// remaining present. A later v2 install is never rediscovered as legacy state.
pub(crate) fn reset(
    connection: &mut Connection,
    roots: &[LegacySkillRoot],
    external_names: &std::collections::BTreeSet<String>,
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
            return initialize_with_names(connection, external_names);
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
    let mut names = files::retired_names(&plan).map_err(|_| rusqlite::Error::InvalidQuery)?;
    names.extend(external_names.iter().cloned());
    initialize_with_names(connection, &names)?;
    crate::logging::info_with_fields(
        "skills.v2.reset",
        "retired durable skill authority; removing owned legacy copies",
        serde_json::json!({"phase":"authority_complete","cleanup_target_count":plan.len()}),
    );
    files::remove(&plan).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("UPDATE durable_state_metadata SET metadata_value=?1 WHERE owner_id=?2 AND metadata_key='filesystem_reset'", params!["complete","skills-v2"])?;
    tx.commit()
}

/// Called with the owning startup connection while runtime admission is closed.
/// Existing events and all checkpoint generations are retired in one FULL-sync
/// SQLite transaction, so an older checkpoint cannot resurrect a grant on retry.
/// Filesystem cleanup and startup admission are a separate phase; callers must
/// not mistake this authority checkpoint for completion of the whole reset.
#[cfg(test)]
pub(super) fn initialize(connection: &mut Connection) -> rusqlite::Result<()> {
    initialize_with_names(connection, &std::collections::BTreeSet::new())
}
fn initialize_with_names(
    connection: &mut Connection,
    external_names: &std::collections::BTreeSet<String>,
) -> rusqlite::Result<()> {
    use rusqlite::{params, OptionalExtension, TransactionBehavior};
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let complete: Option<String> = tx.query_row(
        "SELECT metadata_value FROM durable_state_metadata WHERE owner_id='skills-v2' AND metadata_key='authority_reset'",
        [], |row| row.get(0),
    ).optional()?;
    let publications_complete: Option<String> = tx.query_row(
        "SELECT metadata_value FROM durable_state_metadata WHERE owner_id='skills-v2' AND metadata_key='publication_reference_reset'",
        [], |row| row.get(0),
    ).optional()?;
    let executable_complete: Option<String> = tx.query_row(
        "SELECT metadata_value FROM durable_state_metadata WHERE owner_id='skills-v2' AND metadata_key='executable_database_reset'",
        [], |row| row.get(0),
    ).optional()?;
    if complete.as_deref() == Some("complete")
        && publications_complete.as_deref() == Some("complete")
        && executable_complete.as_deref() == Some("complete")
    {
        return Ok(());
    }
    // A foundation's authority-only checkpoint did not certify publication or
    // entry-intent retirement. Finish those before any runtime admission.
    // Only replayable kernel-owned stores. No App data, credentials, Vault,
    // provider history or transcript database is traversed.
    let mut names = executable::load_names(&tx)?;
    names.extend(external_names.iter().cloned());
    executable::capture_replay_names(&tx, &mut names)?;
    tx.execute("INSERT INTO durable_state_metadata(owner_id,metadata_key,metadata_value,updated_at_ms) VALUES('skills-v2','retired_names',?1,0) ON CONFLICT(owner_id,metadata_key) DO UPDATE SET metadata_value=excluded.metadata_value",[serde_json::to_string(&names).map_err(|_| rusqlite::Error::InvalidQuery)?])?;
    for &(table, kind_column) in REPLAY_TABLES {
        let mut cursor = i64::MIN;
        loop {
            let query = format!(
                "SELECT rowid,{kind_column},payload_json FROM {table} WHERE rowid>?1 ORDER BY rowid LIMIT 128"
            );
            let rows = tx
                .prepare(&query)?
                .query_map([cursor], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if rows.is_empty() {
                break;
            }
            for (id, kind, encoded) in rows {
                cursor = id;
                if !is_replay_authority(table, &kind) {
                    continue;
                }
                // Decode JSON syntax only, never a retired Skill/Grant type.
                let mut value: Value =
                    serde_json::from_str(&encoded).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let before = value.clone();
                if table == "durable_workflow_hot_entities" && kind == "workflow" {
                    executable::retire_definition(&mut value, &names);
                } else {
                    retire_with_names(&mut value, &names);
                }
                if table == "durable_workflow_hot_entities"
                    && kind == "queued_prompt"
                    && queued_selection(&before, &names)
                {
                    tx.execute(&format!("DELETE FROM {table} WHERE rowid=?1"), params![id])?;
                    continue;
                }
                if before != value {
                    if table == "durable_workflow_hot_entities" && kind == "publication_snapshot" {
                        publications::retire_hot(&tx, id)?;
                        continue;
                    }
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
    publications::retire_intents(&tx, None, &names)?;
    for key in [
        "authority_reset",
        "publication_reference_reset",
        "executable_database_reset",
    ] {
        tx.execute("INSERT INTO durable_state_metadata(owner_id,metadata_key,metadata_value,updated_at_ms) VALUES('skills-v2',?1,'complete',0) ON CONFLICT(owner_id,metadata_key) DO UPDATE SET metadata_value='complete'", [key])?;
    }
    tx.commit()
}

// Do not interpret arbitrary event/App data merely because its JSON has an
// extension-shaped field. Only record kinds consumed by runtime restoration
// participate in the destructive reset; unknown payloads stay byte-identical.
fn is_replay_authority(table: &str, kind: &str) -> bool {
    match table {
        "durable_state_events" => matches!(
            kind,
            "session.created"
                | "session.updated"
                | "session.ended"
                | "session.deleted"
                | "sessions.updated"
                | "agents.created"
                | "workflow.publication.materialized"
                | "workflow.publication.reconfigured"
                | "session.prompt_state.updated"
                | "workflow.runtime.updated"
                | "agent.created"
                | "agent.deleted"
                | "agent.updated"
                | "agent.mcp_granted"
                | "agent.mcp_revoked"
                | "agent.skill_granted"
                | "agent.skill_revoked"
                | "agent.extension_granted"
                | "agent.extension_revoked"
                | "agent.runtime_profile_updated"
                | "slice.created"
                | "slice.updated"
                | "slice.deleted"
                | "slice.state.saved"
                | "slice.state.deleted"
                | "slice.backup.created"
                | "slice.backup.restore.started"
                | "slice.backup.restore.committed"
                | "slice.backup.restore.rolled_back"
                | "slice.backup.restore.acknowledged"
        ),
        "durable_state_snapshots" => true,
        "durable_workflow_hot_entities" => matches!(
            kind,
            "workflow" | "queued_prompt" | "publication" | "publication_snapshot" | "schedule"
        ),
        "durable_state_checkpoint_entities" | "durable_state_owner_checkpoint_entities" => {
            matches!(
                kind,
                "sessions"
                    | "agents"
                    | "prompt_private_states"
                    | "slices"
                    | "slice_saved_states"
                    | "slice_backups"
                    | "pending_slice_backup_restores"
                    | "pending_slice_backup_restore_acknowledgements"
            )
        }
        _ => false,
    }
}

fn retire_hidden_context(context: &str) -> String {
    let mut cleaned = context.to_owned();
    for (open, close) in [
        (
            "<skill-context-instructions>",
            "</skill-context-instructions>",
        ),
        ("<chariox_skill", "</chariox_skill>"),
    ] {
        while let Some(start) = cleaned.find(open) {
            let Some(end) = cleaned[start..].find(close) else {
                // A partial component must not leave a replayable instruction
                // body behind. This is hidden execution state, never history.
                return String::new();
            };
            cleaned.replace_range(start..start + end + close.len(), "");
        }
    }
    cleaned
}

fn legacy_grant(value: &Value) -> bool {
    value.get("kind").and_then(Value::as_str) == Some("skill")
}
fn queued_selection(value: &Value, names: &std::collections::BTreeSet<String>) -> bool {
    // Before v2 there is no source identity with which to safely replay these.
    // Cancel explicit selections, including old invocation spelling. Do not
    // reinterpret an unknown $name as a provider-native selection after reset.
    let prompt = value
        .get("prompt")
        .or_else(|| value.get("task_markdown"))
        .and_then(Value::as_str)
        .unwrap_or("");
    crate::skill::selection::explicit_skill_tokens(prompt)
        .iter()
        .any(|token| names.contains(&token.name.to_ascii_lowercase()))
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
#[cfg(test)]
fn retire(value: &mut Value) {
    let mut names = std::collections::BTreeSet::new();
    executable::collect_names(value, &mut names);
    retire_with_names(value, &names);
}
fn retire_with_names(value: &mut Value, names: &std::collections::BTreeSet<String>) {
    match value {
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| retire_with_names(value, names)),
        Value::Object(fields) => {
            publications::retire_session(fields, names);
            for (key, field) in fields.iter_mut() {
                match key.as_str() {
                    "extension_grants" | "extensions" => {
                        if let Some(grants) = field.as_array_mut() {
                            grants.retain(|grant| !legacy_grant(grant));
                        }
                    }
                    "hidden_system_context" => {
                        if let Some(context) = field.as_str() {
                            *field = Value::String(retire_hidden_context(context));
                        }
                    }
                    "skill_grants" | "required_skills" | "skill_selections" => {
                        *field = Value::Array(vec![])
                    }
                    "queued_prompts" | "workflow_queued_prompts" | "queued_metaagent_tasks" => {
                        if let Some(prompts) = field.as_array_mut() {
                            prompts.retain(|prompt| !queued_selection(prompt, names));
                            prompts
                                .iter_mut()
                                .for_each(|prompt| retire_with_names(prompt, names));
                        }
                    }
                    "active_prompt" if queued_selection(field, names) => *field = Value::Null,
                    "workflow" | "workflows" | "definition" => {
                        executable::retire_definition(field, names)
                    }
                    // Typed structural edges only: user JSON, schemas, output and
                    // historical text must never be treated as replay authority.
                    "agent"
                    | "default_agent"
                    | "agents"
                    | "session"
                    | "sessions"
                    | "snapshot"
                    | "requirements"
                    | "nodes"
                    | "prompt_runtime"
                    | "active_prompt"
                    | "prompt_state"
                    | "state"
                    | "private_states"
                    | "prompt_private_states" => retire_with_names(field, names),
                    "prompt_states" => {
                        if let Some(states) = field.as_object_mut() {
                            states
                                .values_mut()
                                .for_each(|state| retire_with_names(state, names));
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
            Self(std::fs::canonicalize(root).unwrap())
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
        reset(&mut db, &root.roots(), &Default::default()).unwrap();
        assert!(!path.exists());
        // New v2 data is outside the retired layout and survives the marker.
        let new = root.0.join("skills/packages/skill-v2/digest");
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("SKILL.md"), "new").unwrap();
        reset(&mut db, &root.roots(), &Default::default()).unwrap();
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
        assert!(reset(&mut db, &root.roots(), &Default::default()).is_err());
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
        let mut value = json!({"agents":[{"extension_grants":[{"kind":"skill","name":"review"}]}],"queued_prompts":[queued.clone(), ordinary.clone()],
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
        let mut selected = json!({"agents":[{"extension_grants":[{"kind":"skill","name":"review"}]}],"queued_prompts":[
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
    // MP-08 / MP-10 / MP-11: reset only runtime replay authority.
    #[test]
    fn skill_reset_preserves_unrelated_application_event_payloads_byte_for_byte() {
        let mut db = database();
        let original = r#"{ "extension_grants": [{"kind":"skill","name":"a field in user data"}], "state": {"required_skills":["documented example"]} }"#;
        db.execute(
            "INSERT INTO durable_state_events VALUES (1,'app','app.user.updated','app-1',1,?1)",
            [original],
        )
        .unwrap();
        initialize(&mut db).unwrap();
        let preserved: String = db
            .query_row(
                "SELECT payload_json FROM durable_state_events WHERE event_id='app'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            preserved, original,
            "application data is not an old instruction authority"
        );
    }
    #[test]
    fn skill_reset_clears_private_instruction_components_without_erasing_other_context() {
        let mut value = json!({"private_states":[{"session_id":"s", "prompt_id":"p",
            "hidden_system_context":"before\n<skill-context-instructions>\nold guidance\n</skill-context-instructions>\nafter",
            "source_user_id":"owner"}],
            "prompt_private_states":[{"session_id":"s", "prompt_id":"p2",
            "hidden_system_context":"<chariox_skill name=\"review\">old body</chariox_skill>\nkeep"}],
            "queued_prompts":[{"id":"p","prompt":"Explain the diff"}],
            "history":[{"hidden_system_context":"<chariox_skill>historical text</chariox_skill>"}]});
        retire(&mut value);
        assert_eq!(
            value["private_states"][0]["hidden_system_context"],
            "before\n\nafter"
        );
        assert_eq!(value["private_states"][0]["source_user_id"], "owner");
        assert_eq!(
            value["prompt_private_states"][0]["hidden_system_context"],
            "\nkeep"
        );
        assert_eq!(value["queued_prompts"][0]["prompt"], "Explain the diff");
        assert_eq!(
            value["history"][0]["hidden_system_context"],
            "<chariox_skill>historical text</chariox_skill>"
        );
    }
    #[test]
    fn skill_reset_drops_malformed_private_instruction_context_instead_of_replaying_a_body() {
        let mut value = json!({"private_states":[{"hidden_system_context":
            "context\n<skill-context-instructions>\nold body without its close"}]});
        retire(&mut value);
        assert_eq!(value["private_states"][0]["hidden_system_context"], "");
    }

    #[test]
    fn skill_reset_retires_batch_and_publication_materialization_events() {
        let mut db = database();
        for (index, kind) in [
            "agents.created",
            "sessions.updated",
            "workflow.publication.materialized",
            "workflow.publication.reconfigured",
        ]
        .iter()
        .enumerate()
        {
            let value=json!({"agents":[{"extension_grants":[{"kind":"skill","name":"review"},{"kind":"mcp","name":"keep"}]}],
                "session":{"prompt_states":{"agent":{"queued_prompts":[{"prompt":"Use $review"}]}}}}).to_string();
            db.execute(
                "INSERT INTO durable_state_events VALUES (?1,?2,?3,'subject',1,?4)",
                rusqlite::params![index as i64, format!("e-{index}"), kind, value],
            )
            .unwrap();
        }
        initialize(&mut db).unwrap();
        let rows = db
            .prepare("SELECT kind,payload_json FROM durable_state_events")
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for (kind, encoded) in rows {
            let value: Value = serde_json::from_str(&encoded).unwrap();
            assert_eq!(
                value["agents"][0]["extension_grants"],
                json!([{"kind":"mcp","name":"keep"}]),
                "{kind}"
            );
            assert_eq!(
                value["session"]["prompt_states"]["agent"]["queued_prompts"],
                json!([]),
                "{kind}"
            );
        }
    }

    // MP-08 / MP-10 / MP-11: immutable publications are invalidated, never rewritten.
    #[test]
    fn skill_reset_invalidates_only_affected_hot_publications() {
        let mut db = database();
        let affected = json!({"agents":[{"extension_grants":[{"kind":"skill","name":"retired"}]}]})
            .to_string();
        let ordinary = r#"{ "agents": [{"extension_grants":[{"kind":"mcp","name":"keep"}]}] }"#;
        for (owner, id, snapshot) in [
            ("owner", "affected", affected.as_str()),
            ("owner", "ordinary", ordinary),
            ("other", "affected", ordinary),
        ] {
            for (kind, payload) in [
                (
                    "publication",
                    json!({"id":id,"enabled":true,"source_snapshot_digest":"immutable"})
                        .to_string(),
                ),
                ("publication_snapshot", snapshot.to_string()),
            ] {
                db.execute(
                    "INSERT INTO durable_workflow_hot_entities VALUES (?1,'session',?2,?3,1,?4)",
                    rusqlite::params![owner, kind, id, payload],
                )
                .unwrap();
            }
        }
        initialize(&mut db).unwrap();
        let count:i64=db.query_row("SELECT count(*) FROM durable_workflow_hot_entities WHERE owner_id='owner' AND entity_id='affected'",[],|r|r.get(0)).unwrap();
        assert_eq!(
            count, 0,
            "do not rewrite a content-addressed publication snapshot under the original digest"
        );
        for (owner, id) in [("owner", "ordinary"), ("other", "affected")] {
            let kept:String=db.query_row("SELECT payload_json FROM durable_workflow_hot_entities WHERE owner_id=?1 AND entity_id=?2 AND entity_kind='publication_snapshot'",rusqlite::params![owner,id],|r|r.get(0)).unwrap();
            assert_eq!(kept, ordinary);
        }
    }
    // MP-08 / MP-10 / MP-11: unstarted selected work has no dispatch intent to retire.
    #[test]
    fn skill_reset_cancels_selected_standalone_hot_queued_prompts_and_meta_tasks() {
        let mut db = database();
        db.execute(
            "INSERT INTO durable_state_events VALUES (1,'e','agent.extension_granted','a',1,?1)",
            [
                json!({"agent":{"extension_grants":[{"kind":"skill","name":"review"}]}})
                    .to_string(),
            ],
        )
        .unwrap();
        for (id, prompt) in [("old", "Use $review"), ("keep", "Explain $total")] {
            db.execute(
                "INSERT INTO durable_workflow_hot_entities VALUES ('owner','session','queued_prompt',?1,1,?2)",
                rusqlite::params![id, json!({"id":id,"prompt":prompt}).to_string()],
            )
            .unwrap();
        }
        initialize(&mut db).unwrap();
        let ids: Vec<String> = db
            .prepare("SELECT entity_id FROM durable_workflow_hot_entities WHERE entity_kind='queued_prompt'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(ids, ["keep"]);
        let mut session = json!({"agents":[{"extension_grants":[{"kind":"skill","name":"review"}]}],
            "queued_metaagent_tasks":[{"id":"m-old","task_markdown":"Use $review here"},
                {"id":"m-keep","task_markdown":"Explain $total"}]});
        retire(&mut session);
        assert_eq!(
            session["queued_metaagent_tasks"],
            json!([{"id":"m-keep","task_markdown":"Explain $total"}])
        );
    }
    #[test]
    fn skill_reset_retires_session_publication_pairs_and_their_pending_invocations() {
        let mut value = json!({"workflow_publications":[{"id":"old"},{"id":"keep"}],
            "workflow_publication_snapshots":{
                "old":{"agents":[{"extension_grants":[{"kind":"skill","name":"retired"}]}]},
                "keep":{"agents":[{"extension_grants":[{"kind":"mcp","name":"keep"}]}]}},
            "workflow_queued_prompts":[{"id":"q-old","publication_invocation":{"publication_id":"old"}},
                {"id":"q-keep","publication_invocation":{"publication_id":"keep"}}],
            "history":[{"workflow_publication_snapshots":{"old":{"immutable":"history"}}}]});
        let kept = value["workflow_publication_snapshots"]["keep"].clone();
        let history = value["history"].clone();
        retire(&mut value);
        assert_eq!(value["workflow_publications"], json!([{"id":"keep"}]));
        assert!(value["workflow_publication_snapshots"].get("old").is_none());
        assert_eq!(value["workflow_publication_snapshots"]["keep"], kept);
        assert_eq!(
            value["workflow_queued_prompts"],
            json!([{"id":"q-keep","publication_invocation":{"publication_id":"keep"}}])
        );
        assert_eq!(value["history"], history);
    }
    #[test]
    fn skill_reset_cancels_skill_bearing_workflow_entry_intents_and_stops_their_runs() {
        let mut db = database();
        db.execute_batch("CREATE TABLE durable_workflow_dispatch_intents(owner_id TEXT,session_id TEXT,run_id TEXT,queued_json TEXT,submitted INTEGER);").unwrap();
        db.execute(
            "INSERT INTO durable_state_snapshots VALUES (1,1,1,?1)",
            [
                json!({"agents":[{"extension_grants":[{"kind":"skill","name":"review"}]}]})
                    .to_string(),
            ],
        )
        .unwrap();
        for (id, prompt) in [("old", "Use $review"), ("keep", "Inspect $HOME/.config")] {
            let queued = json!({"id":id,"prompt":prompt}).to_string();
            let run=json!({"id":id,"status":"Created","active_node_run_id":"n", "node_runs":[{"id":"n","status":"Ready"}],"invocation_prompt":prompt}).to_string();
            db.execute(
                "INSERT INTO durable_workflow_dispatch_intents VALUES ('owner','session',?1,?2,0)",
                rusqlite::params![id, queued],
            )
            .unwrap();
            db.execute("INSERT INTO durable_workflow_runs VALUES ('owner','session',?1,'workflow','Created',1,NULL,1,?2)",rusqlite::params![id,run]).unwrap();
        }
        initialize(&mut db).unwrap();
        let pending: i64 = db
            .query_row(
                "SELECT count(*) FROM durable_workflow_dispatch_intents WHERE run_id='old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            pending, 0,
            "retired queued selection must not replay after startup"
        );
        let (status, encoded): (String, String) = db
            .query_row(
                "SELECT status,payload_json FROM durable_workflow_runs WHERE run_id='old'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "Stopped");
        let value: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value["status"], "Stopped");
        assert_eq!(value["node_runs"][0]["status"], "Stopped");
        assert!(value["active_node_run_id"].is_null());
        let ordinary: i64 = db
            .query_row(
                "SELECT count(*) FROM durable_workflow_dispatch_intents WHERE run_id='keep'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ordinary, 1);
    }

    #[test]
    fn skill_reset_resumes_publication_retirement_after_a_prior_authority_checkpoint() {
        let mut db = database();
        db.execute("INSERT INTO durable_state_metadata VALUES ('skills-v2','authority_reset','complete',0)",[]).unwrap();
        let value=json!({"workflow_publications":[{"id":"old"}],"workflow_publication_snapshots":{"old":{"agents":[{"extension_grants":[{"kind":"skill","name":"old"}]}]}}}).to_string();
        db.execute(
            "INSERT INTO durable_state_events VALUES (1,'e','session.updated','session',1,?1)",
            [value],
        )
        .unwrap();
        initialize(&mut db).unwrap();
        let encoded: String = db
            .query_row("SELECT payload_json FROM durable_state_events", [], |r| {
                r.get(0)
            })
            .unwrap();
        let retired: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(retired["workflow_publications"], json!([]));
        let complete:String=db.query_row("SELECT metadata_value FROM durable_state_metadata WHERE metadata_key='publication_reference_reset'",[],|r|r.get(0)).unwrap();
        assert_eq!(complete, "complete");
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
    fn skill_reset_retires_workflow_invocations_without_erasing_ordinary_variables_or_schema_text()
    {
        let mut db = database();
        let value = json!({
            "agents":[{"extension_grants":[{"kind":"skill","name":"review"}]}],
            "workflows":[{"id":"wf","revision":7,"prompt":"Use $review then explain $total",
                "nodes":[{"instructions":"Read $review then keep $HOME and `$review`"}],
                "schemas":[{"schema":{"default":"$review"}}],
                "code_source":{"source_sha256":"old-binding"}}]
        });
        db.execute(
            "INSERT INTO durable_state_snapshots VALUES(1,1,1,?1)",
            [value.to_string()],
        )
        .unwrap();
        initialize(&mut db).unwrap();
        let encoded: String = db
            .query_row(
                "SELECT payload_json FROM durable_state_snapshots",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let retired: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            retired["workflows"][0]["prompt"],
            "Use  then explain $total"
        );
        assert_eq!(
            retired["workflows"][0]["nodes"][0]["instructions"],
            "Read  then keep $HOME and `$review`"
        );
        assert_eq!(
            retired["workflows"][0]["schemas"],
            value["workflows"][0]["schemas"]
        );
        assert!(retired["workflows"][0]["code_source"].is_null());
        assert_eq!(retired["workflows"][0]["revision"], 8);
    }

    #[test]
    fn skill_reset_detaches_generated_bindings_even_when_only_old_node_grants_change() {
        let mut db = database();
        let value = json!({"workflows":[{"id":"wf","revision":7,"prompt":"Ordinary unchanged text",
            "nodes":[{"extensions":[{"kind":"skill","name":"review"},{"kind":"mcp","name":"safe"}]}],
            "code_source":{"source_sha256":"old-binding"}}]});
        db.execute(
            "INSERT INTO durable_state_snapshots VALUES(1,1,1,?1)",
            [value.to_string()],
        )
        .unwrap();
        initialize(&mut db).unwrap();
        let encoded: String = db
            .query_row(
                "SELECT payload_json FROM durable_state_snapshots",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let retired: Value = serde_json::from_str(&encoded).unwrap();
        assert!(retired["workflows"][0]["code_source"].is_null());
        assert_eq!(
            retired["workflows"][0]["prompt"],
            value["workflows"][0]["prompt"]
        );
        assert_eq!(
            retired["workflows"][0]["nodes"][0]["extensions"],
            json!([{"kind":"mcp","name":"safe"}])
        );
    }

    #[test]
    fn skill_reset_preserves_unresolved_lowercase_dollars_in_queued_and_active_prompts() {
        let mut db = database();
        let ordinary = json!({"id":"keep","prompt":"Why is $total null?"});
        let value = json!({"agents":[{"extension_grants":[{"kind":"skill","name":"review"}]}],
            "queued_prompts":[ordinary.clone(),{"id":"drop","prompt":"Use $review"}],"active_prompt":ordinary.clone()});
        db.execute(
            "INSERT INTO durable_state_snapshots VALUES(1,1,1,?1)",
            [value.to_string()],
        )
        .unwrap();
        initialize(&mut db).unwrap();
        let encoded: String = db
            .query_row(
                "SELECT payload_json FROM durable_state_snapshots",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let retired: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(retired["queued_prompts"], json!([ordinary.clone()]));
        assert_eq!(retired["active_prompt"], ordinary);
    }

    #[test]
    fn skill_reset_invalidates_body_only_publications_instead_of_rewriting_signed_bytes() {
        let mut db = database();
        let value = json!({"agents":[{"extension_grants":[{"kind":"skill","name":"review"}]}],
            "workflow_publications":[{"id":"old","source_snapshot_digest":"immutable"},{"id":"keep"}],
            "workflow_publication_snapshots":{"old":{"workflow":{"prompt":"Use $review"}},"keep":{"workflow":{"prompt":"Explain $total"}}}});
        db.execute(
            "INSERT INTO durable_state_snapshots VALUES(1,1,1,?1)",
            [value.to_string()],
        )
        .unwrap();
        initialize(&mut db).unwrap();
        let encoded: String = db
            .query_row(
                "SELECT payload_json FROM durable_state_snapshots",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let retired: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(retired["workflow_publications"], json!([{"id":"keep"}]));
        assert!(retired["workflow_publication_snapshots"]
            .get("old")
            .is_none());
        assert_eq!(
            retired["workflow_publication_snapshots"]["keep"],
            value["workflow_publication_snapshots"]["keep"]
        );
    }

    #[test]
    fn skill_reset_learns_old_names_before_rewriting_earlier_workflow_rows() {
        let mut db = database();
        for (id, kind, value) in [
            (
                1,
                "session.updated",
                json!({"workflows":[{"id":"wf","prompt":"Use $review"}]}),
            ),
            (
                2,
                "agent.created",
                json!({"agent":{"extension_grants":[{"kind":"skill","name":"review"}]}}),
            ),
        ] {
            db.execute(
                "INSERT INTO durable_state_events VALUES (?1,?2,?3,'s',1,?4)",
                rusqlite::params![id, format!("e-{id}"), kind, value.to_string()],
            )
            .unwrap();
        }
        initialize(&mut db).unwrap();
        let encoded: String = db
            .query_row(
                "SELECT payload_json FROM durable_state_events WHERE sequence=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let retired: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(retired["workflows"][0]["prompt"], "Use ");
        assert!(executable::load_names(&db).unwrap().contains("review"));
    }

    #[test]
    fn skill_reset_known_names_do_not_turn_shell_variables_into_typed_selections() {
        let mut db = database();
        let ordinary = json!({"prompt":"Read $HOME"});
        let before = json!({"agents":[{"extension_grants":[{"kind":"skill","name":"home"}]}],
            "queued_prompts":[ordinary.clone()],"active_prompt":ordinary.clone(),
            "workflows":[{"prompt":"Explain $HOME and use $home"}]});
        db.execute(
            "INSERT INTO durable_state_snapshots VALUES(1,1,1,?1)",
            [before.to_string()],
        )
        .unwrap();
        initialize(&mut db).unwrap();
        let encoded: String = db
            .query_row(
                "SELECT payload_json FROM durable_state_snapshots",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let after: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(after["queued_prompts"], json!([ordinary.clone()]));
        assert_eq!(after["active_prompt"], ordinary);
        assert_eq!(after["workflows"][0]["prompt"], "Explain $HOME and use ");
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
