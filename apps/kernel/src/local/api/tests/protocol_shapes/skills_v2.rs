//! MP-08 / MP-10 / MP-11: Skills v2 wire regression checks; no live acceptance.
use super::*;
use crate::local::CreateProjectRequest;
use crate::skill::v2::*;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};

#[test]
fn skills_v2_protocol_allocation_and_contract_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    assert_eq!(SKILLS_MIN_LOCAL_PROTOCOL, 533);
    assert_eq!(SKILLS_MIN_PEER_PROTOCOL, 141);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_str!("../../../../skill/v2.rs"))
        ),
        "6137dd64e765459d9fcebbccea0f9f556ddb121d1129bae597c1bb4d86457b88"
    );
}

#[test]
fn skills_v2_reset_to_inherited_batch_snapshot() {
    let wire = serde_json::json!({"SkillsV2": {"action":"mutate", "review_receipt_id":"review-1", "mutation": {
        "scope":{"kind":"project","project_id":"p1"}, "expected_revision":4,
        "actions":[{"action":"set_state","skill_id":"s1","state":null}]
    }}});
    let request: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), wire);
    let result = LocalDaemonResponse::SkillsV2 {
        result: SkillsV2Response::Published {
            revision: 5,
            changed_skill_ids: vec!["s1".into()],
            removed_skill_ids: vec![],
        },
    };
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        serde_json::json!({"SkillsV2":{"result":{
            "kind":"published", "revision":5,"changed_skill_ids":["s1"],"removed_skill_ids":[]
        }}})
    );
}

#[test]
fn skills_v2_native_references_reject_client_paths_and_unbound_references() {
    let selection = serde_json::json!({"source":"native","entry_id":"n1","name":"review",
        "provider":"codex","account_profile":"p1","execution_id":"lease1","inventory_revision":"r1"});
    assert!(serde_json::from_value::<SkillSelection>(selection.clone()).is_ok());
    let mut malicious = selection.clone();
    malicious["path"] = serde_json::json!("/client/skill");
    assert!(serde_json::from_value::<SkillSelection>(malicious).is_err());
    for field in [
        "entry_id",
        "provider",
        "account_profile",
        "execution_id",
        "inventory_revision",
    ] {
        let mut unbound = selection.clone();
        unbound.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<SkillSelection>(unbound).is_err());
    }
}

#[test]
fn skills_v2_transfer_requires_revision_manifest_and_bound_receipt() {
    let wire = serde_json::json!({"kind":"ensure_remote_skill_packages", "execution_lease_id":"lease-epoch-1", "context": {
        "home_kernel_id":"home","home_session_id":"s","home_agent_id":"a","leased_agent_id":"lease"
    },"manifest":{"catalog_revision":8,"project_id":"p","skills":[{
        "skill_id":"skill1","digest":"digest1","scope":{"kind":"chariox"},"state":"manual"
    }]},"packages":[]});
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())),
        "604a811e641198ccdc79b461592e6562e08b5c536f80f79711e0922f893ba4e0"
    );
    let request: RelayPeerRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), wire);
    let mut unbound = wire.clone();
    unbound
        .as_object_mut()
        .unwrap()
        .remove("execution_lease_id");
    assert!(serde_json::from_value::<RelayPeerRequest>(unbound).is_err());
    let mut old = wire.clone();
    old.as_object_mut().unwrap().remove("manifest");
    assert!(serde_json::from_value::<RelayPeerRequest>(old).is_err());
    let response = serde_json::json!({"kind":"remote_skill_packages_ensured",
        "context":wire["context"],"execution_lease_id":wire["execution_lease_id"],"manifest":wire["manifest"],"materialized":[]});
    assert_eq!(
        serde_json::to_value(
            serde_json::from_value::<RelayPeerResponse>(response.clone()).unwrap()
        )
        .unwrap(),
        response
    );
}

#[test]
fn skills_v2_unknown_batch_fields_cannot_smuggle_grants() {
    let wire = serde_json::json!({"scope":{"kind":"chariox"},"expected_revision":1,
        "actions":[{"action":"set_state","skill_id":"s","state":"auto","grant":true}]});
    assert!(serde_json::from_value::<SkillCatalogMutation>(wire).is_err());
}

#[test]
fn skills_v2_payload_file_modes_are_explicit_on_the_wire() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let file = crate::skill::CharioxSkillPackageFile {
        path: "helper.sh".into(),
        executable: true,
        sha256: "digest".into(),
        content_base64: "Ynl0ZXM=".into(),
    };
    assert_eq!(
        serde_json::to_value(file).unwrap(),
        serde_json::json!({"path":"helper.sh","executable":true,"sha256":"digest","content_base64":"Ynl0ZXM="})
    );
}

#[test]
fn skills_v2_budget_configuration_wire_snapshot() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let config = crate::config::CharioxUserConfig::default();
    assert_eq!(
        serde_json::to_value(&config).unwrap()["skills"],
        serde_json::json!({"metadata_budget_percent": 1.0})
    );
}

#[test]
fn skills_v2_review_receipt_is_required_for_mutations_and_native_details_are_bound() {
    let old = serde_json::json!({"action":"mutate","mutation":{
        "scope":{"kind":"chariox"},"expected_revision":0,"actions":[]}});
    assert!(serde_json::from_value::<SkillsV2Request>(old.clone()).is_err());
    let mut reviewed = old;
    reviewed["review_receipt_id"] = serde_json::json!("review-1");
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<SkillsV2Request>(reviewed.clone()).unwrap())
            .unwrap(),
        reviewed
    );
    let native = serde_json::json!({"action":"native_details","agent_id":"a", "selection":{
        "source":"native","entry_id":"n","name":"review","provider":"codex",
        "account_profile":"p","execution_id":"e","inventory_revision":"r"}});
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<SkillsV2Request>(native.clone()).unwrap())
            .unwrap(),
        native
    );
    let mut malicious = native;
    malicious["selection"]["path"] = serde_json::json!("/client/choice");
    assert!(serde_json::from_value::<SkillsV2Request>(malicious).is_err());
    let receipt = SkillReviewReceipt {
        id: "review-1".into(),
        expires_at_ms: 10,
        mutation_sha256: "a".repeat(64),
        sources: vec![SkillReviewedSource {
            action_index: 0,
            skill_id: "s".into(),
            digest: "b".repeat(64),
            source_revision: Some("pinned-revision".into()),
        }],
    };
    let wire = serde_json::to_value(SkillsV2Response::Preview {
        expected_revision: 0,
        affected: vec![],
        collisions: vec![],
        review_receipt: receipt,
    })
    .unwrap();
    assert_eq!(
        wire["review_receipt"]["sources"][0]["source_revision"],
        "pinned-revision"
    );
    assert_eq!(
        wire["review_receipt"]["sources"][0]["digest"],
        "b".repeat(64)
    );
}

#[test]
fn skills_v2_folderless_project_protocol_533_snapshot() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let request = LocalDaemonRequest::CreateProject(CreateProjectRequest {
        name: "No folders".into(),
    });
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::json!({"CreateProject":{"name":"No folders"}})
    );
    let project = crate::session::RuntimeProject::new(
        "project-empty",
        "owner",
        "",
        "No folders",
        crate::session::RuntimeProjectKind::Named,
    );
    let wire = serde_json::to_value(LocalDaemonResponse::ProjectCreated { project }).unwrap();
    assert_eq!(
        wire["ProjectCreated"]["project"]["workspace_ids"],
        serde_json::json!([])
    );
    assert_eq!(wire["ProjectCreated"]["project"]["workspace_id"], "");
}

#[test]
fn skills_v2_runtime_capability_is_projected_by_kernel_readiness_not_listed_statically() {
    assert_eq!(
        crate::local::SKILLS_V2_RUNTIME_CAPABILITY,
        "skills_v2_runtime"
    );
    assert!(!crate::local::RUNTIME_CONTROL_CAPABILITIES
        .contains(&crate::local::SKILLS_V2_RUNTIME_CAPABILITY));
    assert_eq!(
        crate::local::SKILLS_REPOSITORY_UPDATE_CAPABILITY,
        "skills_repository_update_v581"
    );
    assert!(!crate::local::RUNTIME_CONTROL_CAPABILITIES
        .contains(&crate::local::SKILLS_REPOSITORY_UPDATE_CAPABILITY));
}

// MP-08 / MP-10 / MP-11: there is no old Skill grant/RPC compatibility decoder.
#[test]
fn skills_v2_removed_grants_and_legacy_rpcs_are_rejected() {
    assert!(
        serde_json::from_value::<crate::extension::ExtensionKind>(serde_json::json!("skill"))
            .is_err()
    );
    for name in [
        "UpsertSkill",
        "InstallSkill",
        "UpdateSkill",
        "UninstallSkill",
        "ImportSkills",
        "GetSkill",
        "ListSkills",
    ] {
        assert!(
            serde_json::from_value::<LocalDaemonRequest>(
                serde_json::json!({name: {"workspace_id":null}})
            )
            .is_err(),
            "{name}"
        );
    }
    let mut old = serde_json::json!({"kind":"submit_leased_prompt", "leased_agent_id":"leased-agent-1", "expected_profile":{
        "provider":"codex", "account_profile":"account-123", "model":"gpt-6-luna", "effort":"low"
    }, "prompt":"hello", "attachments":[]});
    assert!(serde_json::from_value::<RelayPeerRequest>(old.clone()).is_ok());
    old["required_skills"] = serde_json::json!([]);
    assert!(serde_json::from_value::<RelayPeerRequest>(old).is_err());
}

#[test]
fn skills_v2_leased_native_inventory_peer_162_shape_and_hash() {
    assert_eq!(
        RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let wire = serde_json::json!({"kind":"read_leased_skill_native_inventory","context":{
        "home_kernel_id":"home","home_session_id":"s","home_agent_id":"a","leased_agent_id":"lease"
    },"catalog_revision":3,"refresh":false});
    let request: RelayPeerRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), wire);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())),
        "43aaf7f1e1bbc48503665831e03b6d429a58b51bad1b44d07e56b1700221cda7"
    );
    let mut unbound = wire.clone();
    unbound.as_object_mut().unwrap().remove("catalog_revision");
    assert!(serde_json::from_value::<RelayPeerRequest>(unbound).is_err());
    let response = serde_json::json!({"kind":"leased_skill_native_inventory","inventory":{
        "entries":[],"revision":"native-1"}});
    let parsed: RelayPeerResponse = serde_json::from_value(response.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), response);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&response).unwrap())
        ),
        "b42a09ab30a2712a24d766e1c57f036c2aad5f684fda3bd10e538050d4ee5a6b"
    );
    let not_started = serde_json::json!({"kind":"leased_skill_native_inventory","inventory":null});
    assert!(serde_json::from_value::<RelayPeerResponse>(not_started).is_ok());
}

// MP-08 / MP-10 / MP-11: peer 192 adds the worker's compaction report to the inventory.
#[test]
fn skills_v2_leased_native_inventory_compaction_ids_peer_192_shape_and_hash() {
    let response = serde_json::json!({"kind":"leased_skill_native_inventory","inventory":{
        "entries":[],"revision":"native-1","provider_run_id":"run-1",
        "compaction_ids":["compact-1","compact-2"]}});
    let parsed: RelayPeerResponse = serde_json::from_value(response.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), response);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&response).unwrap())
        ),
        "931aa8bed7dbd8147fb0a74020b25a71071bca5ccc8d2d65c512d39441978d96"
    );
    // The field is omitted when empty, which is what a 189 worker sends.
    let RelayPeerResponse::LeasedSkillNativeInventory {
        inventory: Some(old),
        ..
    } = serde_json::from_value(serde_json::json!({"kind":"leased_skill_native_inventory",
            "inventory":{"entries":[],"revision":"native-1"}}))
    .unwrap()
    else {
        panic!("inventory response");
    };
    assert!(old.compaction_ids.is_empty());
}

#[test]
fn skills_v2_ensure_remote_skill_packages_name_view_is_an_additive_peer_179_field() {
    assert_eq!(
        RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let mut wire = serde_json::json!({
        "kind":"ensure_remote_skill_packages","execution_lease_id":"lease",
        "manifest":{"catalog_revision":3,"project_id":null,"skills":[]},
        "context":{"home_kernel_id":"home","home_session_id":"s","home_agent_id":"a","leased_agent_id":"lease-agent"},
        "packages":[]
    });
    // A transfer without typable skills is byte-identical to the one older peers know.
    let request: RelayPeerRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), wire);
    wire["skill_names"] = serde_json::json!({"catalog_revision":3,"skills":[
        {"skill_id":"s1","name":"review","digest":"a".repeat(64)}
    ]});
    let request: RelayPeerRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), wire);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())),
        "f41b15493aea5d3ae9721861dd0317e95783d111510ba9ed3986c159edeff857"
    );
    wire["skill_names"]["skills"][0]["path"] = serde_json::json!("/worker/skill");
    assert!(serde_json::from_value::<RelayPeerRequest>(wire).is_err());
}

// MP-08 / MP-10 / MP-11: the repository update shape is carried by the 581/189 pair.
#[test]
fn skills_v2_repository_update_wire_roundtrips() {
    let wire =
        serde_json::json!({"SkillsV2":{"action":"resolve_repository_update","package_id":"repo"}});
    let request: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), wire);
    let wire = serde_json::json!({"SkillsV2":{"result":{
        "kind":"repository_update", "package_id":"repo", "locator":"https://github.com/owner/skills", "from_revision":"old", "revision":"new",
        "changed":[{"member":{"path":"skills/review","skill_id":"s1","name":"review","digest":"old-digest"},"digest":"new-digest"}],
        "added":[],"removed":[],"unchanged":[],"skipped":[],
        "mutation":{"scope":{"kind":"chariox"},"expected_revision":3,"actions":[]}
    }}});
    let response: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(response).unwrap(), wire);
}
