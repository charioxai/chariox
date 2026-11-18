//! The kernel's catalog validation path for an App drill's calls: every input goes through
//! `AppCatalog::prepare` and every result through `ValidatedToolCall::accept`, the two checks that
//! enforce the 512 KiB call limit for a real window's `chariox.call`. The tools are the App's own
//! `schemas/tools.json`, so a tool that is renamed or reshaped there is refused here too.
use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::{
    app_catalog::{Actor, AppCatalog, CallerContext, CatalogError},
    installation::{
        CapabilityApproval, CapabilityDecision, InstallationRegistry, StageToken,
        VerifiedInstallCandidate,
    },
    publisher_trust::{PublisherTrustRegistry, TrustDecision},
    wire::{Message, Outcome, Success, WIRE_VERSION},
};
use ed25519_dalek::SigningKey;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub struct Catalog {
    connection: Connection,
    catalog: AppCatalog,
    token: StageToken,
}

impl Catalog {
    pub fn new(tools: &[u8]) -> Self {
        let manifest: Manifest = serde_json::from_value(json!({
            "schema":"chariox.app.v1", "appId":"com.example.drill", "version":"1.0.0",
            "publisher":{"id":"com.example", "keyId":"developer", "name":"Developer"},
            "sdkVersion":"0.8.0", "appContractVersion":1, "minKernelProtocol":500,
            "resourcePolicy":"chariox.app.resources.v1", "runtime":{"engine":"node", "entry":"runtime/main.js"},
            "ui":{"entry":"ui/index.html"}, "tools":"schemas/tools.json", "capabilities":{}
        }))
        .unwrap();
        let key = SigningKey::from_bytes(&[39; 32]);
        let files = BTreeMap::from([
            (
                "runtime/main.js".to_string(),
                b"export default function register() {}".to_vec(),
            ),
            (
                "ui/index.html".to_string(),
                b"<!doctype html><title>Drill</title>".to_vec(),
            ),
            ("schemas/tools.json".to_string(), tools.to_vec()),
        ]);
        let publisher = TrustedPublisher {
            publisher_id: "com.example".into(),
            key_id: "developer".into(),
            public_key: key.verifying_key(),
        };
        let bytes = pack(&manifest, &files, &key, &Limits::default()).unwrap();
        let package = verify(
            &bytes,
            &VerificationPolicy::new(500, vec![publisher.clone()]),
        )
        .unwrap();
        let mut connection = Connection::open_in_memory().unwrap();
        InstallationRegistry::new(&mut connection)
            .initialize()
            .unwrap();
        let mut publishers = PublisherTrustRegistry::new(&mut connection);
        publishers.initialize().unwrap();
        let decision = |id: &str| TrustDecision {
            decision_id: id.into(),
            authority_ref: "trusted-kernel-human".into(),
        };
        publishers
            .enroll("owner", &publisher, 0, &decision("enroll"), 1)
            .unwrap();
        let trust = publishers
            .trusted_publisher("owner", &publisher.publisher_id, &publisher.key_id)
            .unwrap();
        let candidate = VerifiedInstallCandidate::from_verified(&package, &trust).unwrap();
        let mut registry = InstallationRegistry::new(&mut connection);
        let token = registry
            .create_and_stage_verified("drill", "owner", &candidate, 1)
            .unwrap()
            .token;
        registry
            .decide(
                &token,
                CapabilityDecision::Approved {
                    approval: CapabilityApproval {
                        decision_id: "approved".into(),
                        authority_ref: "kernel-human".into(),
                    },
                },
                2,
            )
            .unwrap();
        registry.quiesce(&token, 3).unwrap();
        registry.mark_prepared(&token, 4).unwrap();
        registry
            .commit_verified(&token, "owner", &trust, 5)
            .unwrap();
        let binding = InstallationRegistry::new(&mut connection)
            .staged_trust("owner", &token)
            .unwrap();
        let catalog = AppCatalog::compile(&package, &binding, &trust).unwrap();
        Self {
            connection,
            catalog,
            token,
        }
    }

    /// What the kernel does with one call of a window: check the input, then check the App's result.
    pub fn call(
        &mut self,
        tool: &str,
        input: &Value,
        result: &Value,
    ) -> Result<Value, CatalogError> {
        let name = self
            .catalog
            .tools()
            .find(|spec| spec.local_name == tool)
            .unwrap_or_else(|| panic!("the App declares no tool {tool}"))
            .name
            .clone();
        let transaction = self.connection.transaction().unwrap();
        let context = CallerContext {
            actor: Actor::Agent("agent-a".into()),
            room_id: Some("room-a".into()),
            operation_id: "operation-a".into(),
            task_id: Some("task-a".into()),
            turn_id: Some("turn-a".into()),
        };
        let call = self.catalog.prepare(
            &transaction,
            "owner",
            &name,
            input.clone(),
            &context,
            "r",
            1,
            100,
        )?;
        call.accept(
            &transaction,
            "owner",
            Message::Response {
                version: WIRE_VERSION,
                generation: self.token.generation.to_string(),
                id: "r".into(),
                outcome: Outcome::Success(Success {
                    result: result.clone(),
                }),
            },
            2,
        )
    }
}
