// MP-08 / MP-10 / MP-11: the home's name view lives with the lease it was sent for.
use super::*;
use crate::skill::{
    leased_names::{LeasedSkillName, LeasedSkillNames},
    v2::{SkillScope, SkillState, SkillTransferEntry, SkillTransferManifest},
};
use crate::transport::relay_peer::RemoteSkillSyncContext;

struct Transfer {
    context: RemoteSkillSyncContext,
    lease_id: String,
    manifest: SkillTransferManifest,
    package: crate::skill::CharioxSkillPackage,
    names: LeasedSkillNames,
}

fn lease_with_skill(app: &mut DaemonApp) -> Transfer {
    let scratch = std::env::temp_dir().join(format!(
        "chariox-skill-names-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let source = scratch.join("review");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("SKILL.md"),
        "---\nname: review\ndescription: Review code\n---\nReview it.\n",
    )
    .unwrap();
    let package = crate::skill::package_skill_directory(&source).unwrap();
    let _ = std::fs::remove_dir_all(&scratch);
    let mut runtime = RemoteLeaseRuntime::new(app);
    let lease = runtime
        .create_execution_lease(
            "home-kernel",
            &format!("home-session-{}", crate::session::unix_epoch_ms()),
            "home-agent",
            false,
            "local-user",
        )
        .unwrap();
    let leased = runtime
        .create_leased_agent(
            &lease.id, "dev-stub", "default", None, None, None, None, None, None, None,
        )
        .unwrap();
    Transfer {
        context: RemoteSkillSyncContext {
            home_kernel_id: lease.home_kernel_id.clone(),
            home_session_id: lease.home_session_id.clone(),
            home_agent_id: lease.home_agent_id.clone(),
            leased_agent_id: leased.id,
        },
        lease_id: lease.id,
        manifest: SkillTransferManifest {
            catalog_revision: 2,
            project_id: None,
            skills: vec![SkillTransferEntry {
                skill_id: "skill-review".into(),
                digest: package.version_hash.clone(),
                scope: SkillScope::Chariox,
                state: SkillState::Manual,
            }],
        },
        names: LeasedSkillNames {
            catalog_revision: 2,
            skills: vec![LeasedSkillName {
                skill_id: "skill-review".into(),
                name: "review".into(),
                digest: package.version_hash.clone(),
            }],
        },
        package,
    }
}

fn send(
    app: &mut DaemonApp,
    transfer: &Transfer,
    names: Option<LeasedSkillNames>,
) -> Result<(), DaemonError> {
    RemoteLeaseRuntime::new(app)
        .ensure_remote_skill_packages(
            transfer.context.clone(),
            transfer.lease_id.clone(),
            transfer.manifest.clone(),
            vec![transfer.package.clone()],
            names,
        )
        .map(|_| ())
}

fn stored(app: &DaemonApp, transfer: &Transfer) -> Option<LeasedSkillNames> {
    app.leased_agents[&transfer.context.leased_agent_id]
        .skill_names
        .clone()
}

#[test]
fn the_newest_transfer_replaces_the_name_view_of_the_leased_agent() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let transfer = lease_with_skill(&mut app);
    assert_eq!(stored(&app, &transfer), None);
    send(&mut app, &transfer, Some(transfer.names.clone())).unwrap();
    assert_eq!(stored(&app, &transfer), Some(transfer.names.clone()));
    send(&mut app, &transfer, None).unwrap();
    assert_eq!(stored(&app, &transfer), None);
}

#[test]
fn a_name_view_naming_an_untransferred_skill_is_refused_and_keeps_the_previous_view() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let transfer = lease_with_skill(&mut app);
    send(&mut app, &transfer, Some(transfer.names.clone())).unwrap();
    let mut foreign = transfer.names.clone();
    foreign.skills[0].skill_id = "skill-foreign".into();
    assert!(send(&mut app, &transfer, Some(foreign)).is_err());
    assert_eq!(stored(&app, &transfer), Some(transfer.names.clone()));
}
