//! MP-08 / MP-10 / MP-11: owning startup admission, before session restoration or runtime admission.
use super::{
    skill_reset::{self, LegacySkillRoot, LegacySkillRootKind},
    skill_reset_fence,
};
use crate::{config::DaemonConfig, error::DaemonError};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

fn failure() -> DaemonError {
    DaemonError::LocalTransport { operation:"skills.v2.reset", message:"Skills v2 startup cleanup is incomplete; restore owned paths and retry before launching agents".into() }
}
fn absolute(path: &Path) -> Result<PathBuf, DaemonError> {
    if !path.is_absolute() {
        return Err(failure());
    }
    Ok(path.to_path_buf())
}
fn roots(
    config: &DaemonConfig,
    home: &Path,
    global_owner: bool,
) -> Result<(Vec<LegacySkillRoot>, Vec<PathBuf>), DaemonError> {
    let mut legacy = Vec::new();
    if global_owner {
        let mut homes = vec![home.to_path_buf()];
        if let Some(previous) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
            let previous = absolute(&PathBuf::from(previous))?;
            let previous = std::fs::canonicalize(previous).map_err(|_| failure())?;
            homes.push(previous.join(".chariox"));
        }
        for home in homes {
            legacy.push(LegacySkillRoot {
                path: home.join("skills"),
                kind: LegacySkillRootKind::Installed,
            });
            legacy.push(LegacySkillRoot {
                path: home.join("state/skill-materializations"),
                kind: LegacySkillRootKind::Materialized,
            });
            legacy.push(LegacySkillRoot {
                path: home.join("managed-context/kernel-context/user/skills"),
                kind: LegacySkillRootKind::Installed,
            });
        }
    }
    let private = absolute(&config.private_runtime_state_root())?;
    let private = match std::fs::canonicalize(&private) {
        Ok(resolved) => resolved,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => private,
        Err(_) => return Err(failure()),
    };
    legacy.push(LegacySkillRoot {
        path: private.join("skill-materializations"),
        kind: LegacySkillRootKind::Materialized,
    });
    if let Some(isolation) =
        std::env::var_os("CHARIOX_CAPABILITY_ISOLATION_ROOT").filter(|value| !value.is_empty())
    {
        let isolation = absolute(&PathBuf::from(isolation))?;
        let isolation = match std::fs::canonicalize(&isolation) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => isolation,
            Err(_) => return Err(failure()),
        };
        legacy.push(LegacySkillRoot {
            path: isolation.join("user/skills"),
            kind: LegacySkillRootKind::Installed,
        });
        legacy.push(LegacySkillRoot {
            path: isolation.join("state/skill-materializations"),
            kind: LegacySkillRootKind::Materialized,
        });
    }
    let mut seen = BTreeSet::new();
    legacy.retain(|root| {
        seen.insert((
            root.path.clone(),
            matches!(root.kind, LegacySkillRootKind::Installed),
        ))
    });
    let global_artifacts = home.join("workflow-code");
    let private_artifacts = private.join("workflow-code");
    let mut artifacts = Vec::new();
    if global_owner || private_artifacts != global_artifacts {
        artifacts.push(private_artifacts);
    }
    if global_owner {
        artifacts.push(global_artifacts);
    }
    Ok((legacy, artifacts))
}

pub(super) fn run(
    connection: &mut Connection,
    durable: &super::DurableKernelStateStore,
    config: &DaemonConfig,
) -> Result<bool, DaemonError> {
    let home = absolute(config.user_config_path.parent().ok_or_else(failure)?)?;
    std::fs::create_dir_all(&home).map_err(|_| failure())?;
    let home = std::fs::canonicalize(home).map_err(|_| failure())?;
    let (_storage, global_lock, inherited_names) =
        crate::skill::storage::SkillCatalogStorage::startup_scope(&home, durable.clone())?;
    let global_owner = global_lock.is_some();
    let completed:Option<String>=connection.query_row("SELECT metadata_value FROM durable_state_metadata WHERE owner_id='skills-v2' AND metadata_key='executable_reference_reset'",[],|row|row.get(0)).optional().map_err(|_|failure())?;
    if let Some(completed) = completed {
        if completed != "complete" {
            return Err(failure());
        }
        // A crash after the completion checkpoint but before arming the fence
        // only needs the fence. Never reinterpret newly installed v2 sources.
        skill_reset_fence::arm(connection).map_err(|_| failure())?;
        return Ok(global_owner);
    }
    let epoch:Option<String>=connection.query_row("SELECT metadata_value FROM durable_state_metadata WHERE owner_id='skills-v2' AND metadata_key='writer_epoch'",[],|row|row.get(0)).optional().map_err(|_|failure())?;
    if epoch.is_some() {
        return Err(failure());
    }
    let (legacy, artifacts) = roots(config, &home, global_owner)?;
    let mut names = crate::workflow_code::owned_workflow_skill_names(&artifacts)?;
    names.extend(inherited_names);
    skill_reset::reset(connection, &legacy, &names).map_err(|_| failure())?;
    let names = skill_reset::retired_skill_names(connection).map_err(|_| failure())?;
    crate::workflow_code::retire_owned_workflow_skill_artifacts(
        &artifacts,
        &names,
        &config.workflow_code_limits(),
    )?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| failure())?;
    for key in [
        "authority_reset",
        "filesystem_reset",
        "publication_reference_reset",
        "executable_database_reset",
    ] {
        let status:Option<String>=tx.query_row("SELECT metadata_value FROM durable_state_metadata WHERE owner_id='skills-v2' AND metadata_key=?1",[key],|row|row.get(0)).optional().map_err(|_|failure())?;
        if status.as_deref() != Some("complete") {
            return Err(failure());
        }
    }
    tx.execute("INSERT INTO durable_state_metadata(owner_id,metadata_key,metadata_value,updated_at_ms) VALUES('skills-v2','executable_reference_reset','complete',0) ON CONFLICT(owner_id,metadata_key) DO UPDATE SET metadata_value='complete'",[]).map_err(|_|failure())?;
    tx.commit().map_err(|_| failure())?;
    skill_reset_fence::arm(connection).map_err(|_| failure())?;
    Ok(global_owner)
}

#[cfg(test)]
#[path = "skill_reset_startup_tests.rs"]
mod tests;
