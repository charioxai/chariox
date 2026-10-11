//! MP-08 / MP-10 / MP-11: actual runner and contained inventory regressions, supplementary only.
use super::*;
fn context(root:&std::path::Path)->EnvironmentCheckContext {
    EnvironmentCheckContext{config:crate::config::DaemonConfig::for_tests(),owner:"owner".into(),profiles:crate::account_profile::ProviderAccountProfileRegistry::open(root.join("profiles")).unwrap(),apps:crate::durable_state::DurableKernelStateStore::open(root.join("apps.db")).unwrap()}
}
fn operation(env:&ProjectEnvironment,target:&EnvironmentTargetBinding,selected:Vec<&str>)->EnvironmentOperation {
    EnvironmentOperation{operation_id:"review0212-check".into(),attempt:1,local_project_id:env.local_project_id.clone(),revision_digest:env.content_digest.clone(),target:target.clone(),kind:EnvironmentOperationKind::Check,phase:EnvironmentOperationPhase::Requested,selected_items:selected.into_iter().map(str::to_owned).collect(),per_item_opt_ins:vec![],per_item_results:vec![],created_at_ms:crate::session::unix_epoch_ms(),updated_at_ms:crate::session::unix_epoch_ms(),cancellation:None,receipts:vec![],recovery_state:EnvironmentRecoveryState::Pending}
}
fn project(root:&std::path::Path,id:&str)->crate::session::RuntimeProject {
    crate::session::RuntimeProject::new(id,"owner",root.to_str().unwrap(),"Review",crate::session::RuntimeProjectKind::Named)
}
#[test]
fn review0212_partial_check_expires_dependency_journal_evidence() {
    let root=crate::test_support::TestWorktree::new("review0212-expiry");let store=ProjectEnvironmentStore::new(root.path());let project=project(root.path(),"expiry");
    let mut env=store.snapshot(&project).unwrap();let mut draft=environment_draft(&env);
    let mut a=requirement("a");if let RequirementSpec::Software{identity,..}=&mut a.spec {*identity="opaque-runtime".into();}let mut b=requirement("b");b.depends_on=vec!["a".into()];draft.project_requirements=vec![a,b];
    let save=crate::local::SaveProjectEnvironmentRevisionRequest{project_id:project.id().into(),expected_revision:env.revision,expected_content_digest:env.content_digest.clone(),draft,accepted_proposal_ids:vec![],excluded_proposal_ids:vec![]};store.save_revision_locked(&env,&save,"owner").unwrap();env=store.snapshot(&project).unwrap();
    let target=EnvironmentTargetBinding{machine_id:"machine".into(),target_instance_generation:"boot".into(),slice_ref:None};let op=operation(&env,&target,vec!["b"]);
    let mut journal=EnvironmentChecks::default();journal.folder_binding_digest=folder_bindings_digest(&env);journal.observations=vec![observation(&env,"a",&target,crate::session::unix_epoch_ms()-super::super::check_store::OBSERVATION_TTL_MS-1)];journal.operations.push(op.clone());store.save_checks(project.id(),&journal).unwrap();
    store.run_check(context(root.path()),env,op,||Ok(project.clone())).unwrap();let journal=store.load_checks(project.id()).unwrap();
    assert!(journal.observations.iter().find(|o|o.requirement_id=="a").is_some_and(|a|a.stale || a.status!=EnvironmentObservationStatus::Ready),"expired prerequisite must not certify a fresh dependent");
    assert_eq!(journal.observations.iter().find(|o|o.requirement_id=="b").unwrap().status,EnvironmentObservationStatus::NeedsYourInput);
}
