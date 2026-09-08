use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::{
    app_catalog::{
        Actor, AppCatalog, CallerContext, CatalogError, MAX_CALL_BYTES, MAX_CALL_NODES,
        MAX_TOOL_NAME_BYTES,
    },
    installation::{
        CapabilityApproval, CapabilityDecision, InstallationRegistry, StageToken, UpdatePhase,
        VerifiedInstallCandidate,
    },
    publisher_trust::{PublisherTrustRegistry, TrustDecision, TrustedPublisherSnapshot},
    wire::{Failure, Message, Outcome, RemoteError, Success, WIRE_VERSION},
};
use ed25519_dalek::SigningKey;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[path = "app_catalog/owned.rs"]
mod owned;
#[path = "app_catalog/readiness.rs"]
mod readiness;

struct Package {
    manifest: Manifest,
    files: BTreeMap<String, Vec<u8>>,
    key: SigningKey,
}
impl Package {
    fn new() -> Self {
        Self {
            manifest: serde_json::from_value(json!({
                "schema":"chariox.app.v1", "appId":"com.example.catalog", "version":"1.0.0",
                "publisher":{"id":"com.example", "keyId":"developer", "name":"Developer"},
                "sdkVersion":"0.7.0", "appContractVersion":1, "minKernelProtocol":500,
                "resourcePolicy":"chariox.app.resources.v1", "runtime":{"engine":"node", "entry":"runtime/main.js"},
                "ui":{"entry":"ui/index.html"}, "tools":"schemas/tools.json", "capabilities":{}
            })).unwrap(),
            key: SigningKey::from_bytes(&[39; 32]),
            files: BTreeMap::from([
                ("runtime/main.js".into(), b"export default function register() {}".to_vec()),
                ("ui/index.html".into(), b"<!doctype html><title>Catalog fixture</title>".to_vec()),
                ("schemas/tools.json".into(), serde_json::to_vec(&json!({"tools":[{
                    "name":"echo", "description":"Return the text", "inputSchema":text_schema(),
                    "outputSchema":text_schema()
                }]})).unwrap()),
            ]),
        }
    }
    fn publisher(&self) -> TrustedPublisher {
        TrustedPublisher {
            publisher_id: self.manifest.publisher.id.clone(),
            key_id: self.manifest.publisher.key_id.clone(),
            public_key: self.key.verifying_key(),
        }
    }
    fn bytes(&self) -> Vec<u8> {
        pack(&self.manifest, &self.files, &self.key, &Limits::default()).unwrap()
    }
    fn candidate(&self, trust: &TrustedPublisherSnapshot) -> VerifiedInstallCandidate {
        let bytes = self.bytes();
        let package = verify(
            &bytes,
            &VerificationPolicy::new(500, vec![self.publisher()]),
        )
        .unwrap();
        VerifiedInstallCandidate::from_verified(&package, trust).unwrap()
    }
    fn catalog(
        &self,
        connection: &mut Connection,
        token: &StageToken,
        trust: &TrustedPublisherSnapshot,
    ) -> Result<AppCatalog, CatalogError> {
        let bytes = self.bytes();
        let package = verify(
            &bytes,
            &VerificationPolicy::new(500, vec![self.publisher()]),
        )
        .unwrap();
        let binding = InstallationRegistry::new(connection)
            .staged_trust("owner", token)
            .unwrap();
        AppCatalog::compile(&package, &binding, trust)
    }
}
fn text_schema() -> Value {
    json!({"type":"object", "additionalProperties":false,
    "properties":{"text":{"type":"string"}}, "required":["text"]})
}
fn decision(id: &str) -> TrustDecision {
    TrustDecision {
        decision_id: id.into(),
        authority_ref: "trusted-kernel-human".into(),
    }
}
fn database(package: &Package) -> (Connection, TrustedPublisherSnapshot) {
    let mut connection = Connection::open_in_memory().unwrap();
    InstallationRegistry::new(&mut connection)
        .initialize()
        .unwrap();
    let mut publishers = PublisherTrustRegistry::new(&mut connection);
    publishers.initialize().unwrap();
    let publisher = package.publisher();
    publishers
        .enroll("owner", &publisher, 0, &decision("enroll"), 1)
        .unwrap();
    let trust = publishers
        .trusted_publisher("owner", &publisher.publisher_id, &publisher.key_id)
        .unwrap();
    (connection, trust)
}
fn install(
    connection: &mut Connection,
    id: &str,
    package: &Package,
    trust: &TrustedPublisherSnapshot,
) -> StageToken {
    let token = InstallationRegistry::new(connection)
        .create_and_stage_verified(id, "owner", &package.candidate(trust), 1)
        .unwrap()
        .token;
    activate(connection, &token, trust);
    token
}
fn activate(connection: &mut Connection, token: &StageToken, trust: &TrustedPublisherSnapshot) {
    let mut registry = InstallationRegistry::new(connection);
    registry
        .decide(
            token,
            CapabilityDecision::Approved {
                approval: CapabilityApproval {
                    decision_id: format!("approved-{}-{}", token.installation_id, token.generation),
                    authority_ref: "kernel-human".into(),
                },
            },
            2,
        )
        .unwrap();
    registry.quiesce(token, 3).unwrap();
    registry.mark_prepared(token, 4).unwrap();
    registry.commit_verified(token, "owner", trust, 5).unwrap();
}
fn context() -> CallerContext {
    CallerContext {
        actor: Actor::Agent("agent-a".into()),
        room_id: "room-a".into(),
        operation_id: "operation-a".into(),
        task_id: Some("task-a".into()),
        turn_id: Some("turn-a".into()),
    }
}
fn response(id: &str, generation: u64, result: Value) -> Message {
    Message::Response {
        version: WIRE_VERSION,
        generation: generation.to_string(),
        id: id.into(),
        outcome: Outcome::Success(Success { result }),
    }
}

#[test]
fn names_are_stable_across_updates_and_distinguish_installations_and_full_local_names() {
    let mut package = Package::new();
    let local_a = format!("{}a", "same_readable_local_prefix_".repeat(2));
    let local_b = format!("{}b", "same_readable_local_prefix_".repeat(2));
    package.files.insert("schemas/tools.json".into(), serde_json::to_vec(&json!({"tools":[
        {"name":local_a, "inputSchema":text_schema()}, {"name":local_b,"inputSchema":text_schema()}
    ]})).unwrap());
    let (mut connection, trust) = database(&package);
    let first_token = install(&mut connection, "first-installation", &package, &trust);
    let second_token = install(&mut connection, "second-installation", &package, &trust);
    let first = package
        .catalog(&mut connection, &first_token, &trust)
        .unwrap();
    let second = package
        .catalog(&mut connection, &second_token, &trust)
        .unwrap();
    let names: Vec<_> = first.tools().map(|tool| tool.name.clone()).collect();
    assert_ne!(names[0], names[1]);
    assert!(names.iter().all(|name| name.len() <= MAX_TOOL_NAME_BYTES
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')));
    assert!(second.tools().all(|tool| !names.contains(&tool.name)));
    assert!(first
        .tools()
        .all(|tool| tool.description.contains("com.example.catalog")
            && tool.description.contains("first-installation")));
    assert!(matches!(
        first.check_names([names[0].as_str()]),
        Err(CatalogError::NameCollision)
    ));
    first.check_names(["read_artifact", "echo"]).unwrap();
    assert!(first.tool(&format!("chariox_{}", names[0])).is_none());
    assert!(first.tool(&format!("mcp__chariox__{}", names[0])).is_none());
    package.manifest.version = "2.0.0".into();
    let updated = InstallationRegistry::new(&mut connection)
        .stage_verified(
            "first-installation",
            "owner",
            first_token.generation,
            &package.candidate(&trust),
            10,
        )
        .unwrap()
        .token;
    activate(&mut connection, &updated, &trust);
    let next = package.catalog(&mut connection, &updated, &trust).unwrap();
    assert_eq!(
        names,
        next.tools()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>()
    );
    let transaction = connection.transaction().unwrap();
    assert!(first.require_current(&transaction, "owner").is_err());
    next.require_current(&transaction, "owner").unwrap();
}

#[test]
fn package_and_exact_signer_are_bound_to_installed_release() {
    let mut package = Package::new();
    let (mut connection, trust) = database(&package);
    let token = install(&mut connection, "installed", &package, &trust);
    package.files.insert(
        "runtime/main.js".into(),
        b"export default function changed() {}".to_vec(),
    );
    assert!(matches!(
        package.catalog(&mut connection, &token, &trust),
        Err(CatalogError::Provenance)
    ));
    package.key = SigningKey::from_bytes(&[40; 32]);
    assert!(package.catalog(&mut connection, &token, &trust).is_err());
}

#[test]
fn critical_action_metadata_is_preserved_for_the_common_kernel_policy() {
    let mut package = Package::new();
    package.manifest.actions = Some("schemas/actions.json".into());
    package.manifest.capabilities.network = serde_json::from_value(json!([
        {"origin":"https://api.example.com", "methods":["POST"]}
    ]))
    .unwrap();
    let action = json!({"name":"send", "inputSchema":text_schema(),
        "criticalValidation":{"reason":"Confirm this external send", "userVerification":true},
        "effectRoutes":[{"origin":"https://api.example.com", "method":"POST", "path":"/send", "connection":"service"}]});
    package.files.insert(
        "schemas/actions.json".into(),
        serde_json::to_vec(&json!({"actions":[action]})).unwrap(),
    );
    package.files.insert(
        "schemas/tools.json".into(),
        serde_json::to_vec(&json!({"tools":[
            {"name":"send", "inputSchema":text_schema(), "action":"send"}
        ]}))
        .unwrap(),
    );
    let (mut connection, trust) = database(&package);
    let token = install(&mut connection, "installed", &package, &trust);
    let catalog = package.catalog(&mut connection, &token, &trust).unwrap();
    let tool = catalog.tools().next().unwrap();
    let declared = tool.action.as_ref().unwrap();
    assert_eq!(declared.name, "send");
    assert!(
        declared
            .critical_validation
            .as_ref()
            .unwrap()
            .user_verification
    );
    assert_eq!(declared.effect_routes[0].path, "/send");
    assert_eq!(declared.input_schema, tool.input_schema);
}

#[test]
fn input_schema_and_typed_context_precede_worker_dispatch() {
    let package = Package::new();
    let (mut connection, trust) = database(&package);
    let token = install(&mut connection, "installed", &package, &trust);
    let catalog = package.catalog(&mut connection, &token, &trust).unwrap();
    let name = catalog.tools().next().unwrap().name.clone();
    let transaction = connection.transaction().unwrap();
    for input in [
        json!({}),
        json!({"text":2}),
        json!({"text":"ok", "extra":true}),
        json!(["text"]),
    ] {
        assert!(matches!(
            catalog.prepare(
                &transaction,
                "owner",
                &name,
                input,
                &context(),
                "request-1",
                10,
                100
            ),
            Err(CatalogError::Input)
        ));
    }
    assert!(matches!(
        catalog.prepare(
            &transaction,
            "owner",
            "echo",
            json!({"text":"ok"}),
            &context(),
            "request-1",
            10,
            100
        ),
        Err(CatalogError::UnknownTool)
    ));
    assert!(catalog
        .prepare(
            &transaction,
            "other-owner",
            &name,
            json!({"text":"ok"}),
            &context(),
            "request-1",
            10,
            100
        )
        .is_err());
    let call = catalog
        .prepare(
            &transaction,
            "owner",
            &name,
            json!({"text":"ok"}),
            &context(),
            "request-1",
            10,
            100,
        )
        .unwrap();
    let Message::Request {
        method,
        params,
        context,
        ..
    } = call.request()
    else {
        panic!("request required")
    };
    assert_eq!(method, "tools.invoke");
    assert_eq!(params, &json!({"name":"echo", "input":{"text":"ok"}}));
    assert_eq!(
        context,
        &Some(
            json!({"installation_id":"installed", "room_id":"room-a", "operation_id":"operation-a",
        "actor":{"kind":"agent", "id":"agent-a"}, "agent_id":"agent-a", "task_id":"task-a", "turn_id":"turn-a"})
        )
    );
    call.accept(
        &transaction,
        "owner",
        response("request-1", token.generation, json!({"text":"ok"})),
        20,
    )
    .unwrap();
}

#[test]
fn correlation_deadlines_and_success_schemas_are_checked_even_for_typed_responses() {
    let package = Package::new();
    let (mut connection, trust) = database(&package);
    let token = install(&mut connection, "installed", &package, &trust);
    let catalog = package.catalog(&mut connection, &token, &trust).unwrap();
    let name = catalog.tools().next().unwrap().name.clone();
    let transaction = connection.transaction().unwrap();
    let prepare = || {
        catalog
            .prepare(
                &transaction,
                "owner",
                &name,
                json!({"text":"ok"}),
                &context(),
                "request-1",
                10,
                100,
            )
            .unwrap()
    };
    assert!(matches!(
        prepare().accept(
            &transaction,
            "owner",
            response("other", token.generation, json!({"text":"ok"})),
            20
        ),
        Err(CatalogError::Response)
    ));
    assert!(matches!(
        prepare().accept(
            &transaction,
            "owner",
            response("request-1", token.generation + 1, json!({"text":"ok"})),
            20
        ),
        Err(CatalogError::Response)
    ));
    assert!(matches!(
        prepare().accept(
            &transaction,
            "owner",
            response("request-1", token.generation, json!({"text":"ok"})),
            100
        ),
        Err(CatalogError::Deadline)
    ));
    assert!(matches!(
        prepare().accept(
            &transaction,
            "owner",
            response("request-1", token.generation, json!({"text":42})),
            20
        ),
        Err(CatalogError::Output)
    ));
    assert!(matches!(
        prepare().accept(
            &transaction,
            "owner",
            Message::Event {
                version: WIRE_VERSION,
                generation: token.generation.to_string(),
                name: "worker.done".into(),
                data: json!({"text":"ok"})
            },
            20
        ),
        Err(CatalogError::Response)
    ));
    assert!(matches!(
        prepare().accept(
            &transaction,
            "owner",
            Message::Response {
                version: WIRE_VERSION,
                generation: token.generation.to_string(),
                id: "request-1".into(),
                outcome: Outcome::Failure(Failure {
                    error: RemoteError {
                        code: "FAILED".into(),
                        message: "bounded untrusted App detail".into(),
                        retryable: Some(false)
                    }
                })
            },
            20
        ),
        Err(CatalogError::Worker(_))
    ));
    for deadline in [0, 10, 30_011] {
        assert!(matches!(
            catalog.prepare(
                &transaction,
                "owner",
                &name,
                json!({"text":"ok"}),
                &context(),
                "request-1",
                10,
                deadline
            ),
            Err(CatalogError::Deadline)
        ));
    }
}

#[test]
fn payload_limits_apply_without_optional_output_schema() {
    let mut package = Package::new();
    package.files.insert(
        "schemas/tools.json".into(),
        serde_json::to_vec(&json!({"tools":[{"name":"echo", "inputSchema":text_schema()}]}))
            .unwrap(),
    );
    let (mut connection, trust) = database(&package);
    let token = install(&mut connection, "installed", &package, &trust);
    let catalog = package.catalog(&mut connection, &token, &trust).unwrap();
    let name = catalog.tools().next().unwrap().name.clone();
    let transaction = connection.transaction().unwrap();
    assert!(matches!(
        catalog.prepare(
            &transaction,
            "owner",
            &name,
            json!({"text":"x".repeat(MAX_CALL_BYTES)}),
            &context(),
            "r",
            1,
            100
        ),
        Err(CatalogError::Limit)
    ));
    let prepare = || {
        catalog
            .prepare(
                &transaction,
                "owner",
                &name,
                json!({"text":"ok"}),
                &context(),
                "r",
                1,
                100,
            )
            .unwrap()
    };
    assert!(matches!(
        prepare().accept(
            &transaction,
            "owner",
            response("r", token.generation, json!("x".repeat(MAX_CALL_BYTES))),
            2
        ),
        Err(CatalogError::Limit)
    ));
    assert!(matches!(
        prepare().accept(
            &transaction,
            "owner",
            response("r", token.generation, json!(9_007_199_254_740_992_u64)),
            2
        ),
        Err(CatalogError::Invalid)
    ));
    for value in [
        Value::Array(vec![Value::Null; MAX_CALL_NODES]),
        json!({("x".repeat(MAX_CALL_BYTES)): null}),
    ] {
        assert!(matches!(
            prepare().accept(
                &transaction,
                "owner",
                response("r", token.generation, value),
                2
            ),
            Err(CatalogError::Limit)
        ));
    }
    let mut deep = Value::Null;
    for _ in 0..63 {
        deep = json!([deep]);
    }
    assert!(matches!(
        prepare().accept(
            &transaction,
            "owner",
            response("r", token.generation, deep),
            2
        ),
        Err(CatalogError::Limit)
    ));
    assert_eq!(
        prepare()
            .accept(
                &transaction,
                "owner",
                response("r", token.generation, Value::Null),
                2
            )
            .unwrap(),
        Value::Null
    );
}

#[test]
fn same_generation_publisher_revocation_blocks_catalog_and_in_flight_result() {
    let package = Package::new();
    let (mut connection, trust) = database(&package);
    let token = install(&mut connection, "installed", &package, &trust);
    let catalog = package.catalog(&mut connection, &token, &trust).unwrap();
    let name = catalog.tools().next().unwrap().name.clone();
    let transaction = connection.transaction().unwrap();
    let call = catalog
        .prepare(
            &transaction,
            "owner",
            &name,
            json!({"text":"ok"}),
            &context(),
            "r",
            1,
            100,
        )
        .unwrap();
    transaction.commit().unwrap();
    PublisherTrustRegistry::new(&mut connection)
        .revoke(
            "owner",
            "com.example",
            "developer",
            trust.revision(),
            &decision("revoke"),
            20,
        )
        .unwrap();
    assert_eq!(
        InstallationRegistry::new(&mut connection)
            .get("installed")
            .unwrap()
            .generation,
        token.generation
    );
    let transaction = connection.transaction().unwrap();
    assert!(catalog.require_current(&transaction, "owner").is_err());
    assert!(call
        .accept(
            &transaction,
            "owner",
            response(
                "r",
                token.generation,
                json!({"text":"effect-may-have-happened"})
            ),
            21
        )
        .is_err());
    transaction.commit().unwrap();
    let mut publishers = PublisherTrustRegistry::new(&mut connection);
    let revoked = publishers.get("owner", "com.example", "developer").unwrap();
    publishers
        .enroll(
            "owner",
            &package.publisher(),
            revoked.revision,
            &decision("reenroll"),
            22,
        )
        .unwrap();
    let fresh = publishers
        .trusted_publisher("owner", "com.example", "developer")
        .unwrap();
    assert!(matches!(
        package.catalog(&mut connection, &token, &fresh),
        Err(CatalogError::Provenance)
    ));
    assert!(catalog
        .require_current(&connection.transaction().unwrap(), "owner")
        .is_err());
}

#[test]
fn quiescence_and_uninstall_fence_admission_without_changing_a_prepared_calls_context() {
    let mut package = Package::new();
    let (mut connection, trust) = database(&package);
    let token = install(&mut connection, "installed", &package, &trust);
    let catalog = package.catalog(&mut connection, &token, &trust).unwrap();
    let name = catalog.tools().next().unwrap().name.clone();
    let transaction = connection.transaction().unwrap();
    let mut caller = context();
    let call = catalog
        .prepare(
            &transaction,
            "owner",
            &name,
            json!({"text":"ok"}),
            &caller,
            "r",
            1,
            100,
        )
        .unwrap();
    transaction.commit().unwrap();
    caller.actor = Actor::Agent("agent-b".into());
    let Message::Request { context, .. } = call.request() else {
        panic!("request")
    };
    assert_eq!(context.as_ref().unwrap()["agent_id"], "agent-a");
    package.manifest.version = "2.0.0".into();
    let mut registry = InstallationRegistry::new(&mut connection);
    let next = registry
        .stage_verified(
            "installed",
            "owner",
            token.generation,
            &package.candidate(&trust),
            10,
        )
        .unwrap()
        .token;
    registry
        .decide(
            &next,
            CapabilityDecision::Approved {
                approval: CapabilityApproval {
                    decision_id: "update".into(),
                    authority_ref: "kernel-human".into(),
                },
            },
            11,
        )
        .unwrap();
    registry.quiesce(&next, 12).unwrap();
    let transaction = connection.transaction().unwrap();
    assert!(catalog.require_current(&transaction, "owner").is_err());
    assert!(call
        .accept(
            &transaction,
            "owner",
            response("r", token.generation, json!({"text":"ok"})),
            13
        )
        .is_err());
    transaction.commit().unwrap();
    let mut registry = InstallationRegistry::new(&mut connection);
    assert_eq!(
        registry.abort(&next, "test abort", 14).unwrap().phase,
        UpdatePhase::Aborted
    );
    let transaction = connection.transaction().unwrap();
    catalog.require_current(&transaction, "owner").unwrap();
    transaction.commit().unwrap();
    InstallationRegistry::new(&mut connection)
        .uninstall("installed", token.generation, 15)
        .unwrap();
    assert!(catalog
        .require_current(&connection.transaction().unwrap(), "owner")
        .is_err());
}
