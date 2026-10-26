//! Frozen Apps v2 contracts (docs/APPS_V2_CONTRACTS.md §3–§7) against shared
//! vectors. A failure means an encoding or rule changed: update the contract
//! and the vector together, with `CHARIOX_UPDATE_VECTORS=1` for the vector.
use std::{fs, path::PathBuf};

use chariox_app_package::{v2::SignerPrincipal, ActionDeclaration, Capabilities, Publisher};
use chariox_app_runtime::{
    installation::{
        activation::{
            activation_entry, approval_requirement, check_source, commit_gate, AuthoritySet,
        },
        channel::{
            check_immutable_version, resolve, AcquisitionSource, InstallationChannel,
            InstallationRef, Selector, UpdateChannel,
        },
    },
    managed_state::data_contract::{accept_migration, recovery, UpdateStage},
    release_store::aot_key::AotCacheKey,
};
use serde_json::{json, Value};

fn check_vectors(file: &str, run: impl Fn(&Value, &Value) -> Value) {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests/vectors", file]
        .iter()
        .collect();
    let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let shared = document.get("shared").cloned().unwrap_or(Value::Null);
    let update = std::env::var_os("CHARIOX_UPDATE_VECTORS").is_some();
    for case in document["cases"].as_array_mut().unwrap() {
        let actual = run(&shared, &case["input"]);
        if update {
            case["expected"] = actual;
        } else {
            assert!(
                case["expected"] == actual,
                "{file} / {}: contract changed\nexpected {:#}\nactual   {actual:#}",
                case["name"],
                case["expected"]
            );
        }
    }
    if update {
        fs::write(
            &path,
            serde_json::to_string_pretty(&document).unwrap() + "\n",
        )
        .unwrap();
    }
}

fn text(value: &Value) -> String {
    value.as_str().unwrap().to_owned()
}

fn word<T: serde::de::DeserializeOwned>(value: &Value) -> T {
    serde_json::from_value(value.clone()).unwrap()
}

#[test]
fn channel_resolution_vectors() {
    check_vectors("channel-resolution.json", |shared, input| {
        let installations: Vec<_> = shared["installations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| InstallationRef {
                installation_id: text(&item["installationId"]),
                owner_id: text(&item["ownerId"]),
                app_id: text(&item["appId"]),
                channel: word(&item["channel"]),
                dev_room_id: item["devRoomId"].as_str().map(str::to_owned),
                update_channel: item.get("updateChannel").map(word::<UpdateChannel>),
            })
            .collect();
        for item in &installations {
            item.validate().unwrap();
        }
        let selector = Selector {
            owner_id: text(&input["ownerId"]),
            app_id: text(&input["appId"]),
            room_id: input["roomId"].as_str().map(str::to_owned),
            channel: input.get("channel").map(word::<InstallationChannel>),
        };
        serde_json::to_value(resolve(&installations, &selector)).unwrap()
    });
}

#[test]
fn a_dev_installation_needs_a_room_and_no_subscription() {
    let dev = |room: Option<&str>, update_channel| InstallationRef {
        installation_id: "i".to_owned(),
        owner_id: "o".to_owned(),
        app_id: "com.example.todo".to_owned(),
        channel: InstallationChannel::Dev,
        dev_room_id: room.map(str::to_owned),
        update_channel,
    };
    assert!(dev(None, None).validate().is_err());
    assert!(dev(Some("r"), Some(UpdateChannel::Stable))
        .validate()
        .is_err());
    let release = InstallationRef {
        channel: InstallationChannel::Release,
        ..dev(Some("r"), None)
    };
    assert!(release.validate().is_err());
}

#[test]
fn activation_routing_vectors() {
    check_vectors("activation-routing.json", |_, input| {
        match input["kind"].as_str().unwrap() {
            "entry" => serde_json::to_value(activation_entry(input["existingGeneration"].as_u64()))
                .unwrap(),
            "source" => {
                let signer = SignerPrincipal::of(&Publisher {
                    id: if input["signer"] == "dev" {
                        "local_dev"
                    } else {
                        "com.example"
                    }
                    .to_owned(),
                    key_id: "k".to_owned(),
                    name: "n".to_owned(),
                });
                let source: AcquisitionSource = word(&input["source"]);
                match check_source(source, word(&input["channel"]), &signer) {
                    Ok(()) => json!({"allowed": true}),
                    Err(refusal) => json!({"refusal": refusal}),
                }
            }
            _ => match check_immutable_version(
                "com.example.todo",
                input["version"].as_str().unwrap(),
                input["known"].as_str(),
                input["incoming"].as_str().unwrap(),
            ) {
                Ok(()) => json!({"allowed": true}),
                Err(conflict) => json!({"conflict": conflict.to_string()}),
            },
        }
    });
}

#[test]
fn authority_vectors() {
    check_vectors("authority.json", |_, input| {
        match input["kind"].as_str().unwrap() {
            "authority" => {
                let capabilities: Capabilities = word(&input["capabilities"]);
                let actions: Vec<ActionDeclaration> = word(&input["actions"]);
                let set = AuthoritySet::of(&capabilities, &actions);
                json!({"atoms": set.atoms().collect::<Vec<_>>(), "digest": set.digest()})
            }
            "requirement" => {
                let approved = input["approved"]
                    .is_array()
                    .then(|| word(&input["approved"]));
                serde_json::to_value(approval_requirement(
                    approved.as_ref(),
                    &word::<AuthoritySet>(&input["candidate"]),
                ))
                .unwrap()
            }
            _ => match commit_gate(
                &word(&input["approvedNow"]),
                &word(&input["candidate"]),
                input["decidedTrustRevision"].as_u64().unwrap(),
                input["trustRevisionNow"].as_u64().unwrap(),
            ) {
                Ok(()) => json!({"allowed": true}),
                Err(refusal) => serde_json::to_value(refusal).unwrap(),
            },
        }
    });
}

#[test]
fn aot_key_vectors() {
    check_vectors("aot-key.json", |_, input| {
        let entry: AotCacheKey = word(&input["entry"]);
        let running: AotCacheKey = word(&input["running"]);
        let provenance = chariox_app_runtime::release_store::aot_key::AotProvenance {
            key: entry.clone(),
            source_release_digest: "sha256:r0".to_owned(),
            artifact_digest: "sha256:a0".to_owned(),
        };
        json!({
            "entryCanonical": String::from_utf8(entry.canonical_bytes()).unwrap(),
            "entryId": entry.id(),
            "runningId": running.id(),
            "usable": provenance.usable_by(&running),
        })
    });
}

#[test]
fn migration_vectors() {
    check_vectors("migration.json", |_, input| {
        match input["kind"].as_str().unwrap() {
            "accept" => serde_json::to_value(accept_migration(
                input["dataVersion"].as_u64().unwrap() as u32,
                input["releaseTarget"].as_u64().unwrap() as u32,
            ))
            .unwrap(),
            _ => {
                let stage = match input["stage"].as_str().unwrap() {
                    "before_commit" => UpdateStage::BeforeCommit,
                    _ => UpdateStage::AfterCommit,
                };
                serde_json::to_value(recovery(stage, input["compatibleMigration"] == true)).unwrap()
            }
        }
    });
}
