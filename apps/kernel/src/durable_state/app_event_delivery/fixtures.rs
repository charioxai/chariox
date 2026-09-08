use super::*;
use crate::session::SessionService;
use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::{
    app_catalog::AppCatalog,
    installation::{
        CapabilityApproval, CapabilityDecision, InstallationRegistry, VerifiedInstallCandidate,
    },
    publisher_trust::{PublisherTrustRegistry, TrustDecision},
};
use ed25519_dalek::SigningKey;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) catalog: Arc<EventCatalog>,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("chariox-app-event-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        // Test-only enrolled publisher/verified installation. Production obtains
        // these through the existing installer/permission authority.
        let mut db = Connection::open(root.join("kernel.sqlite")).unwrap();
        InstallationRegistry::new(&mut db).initialize().unwrap();
        PublisherTrustRegistry::new(&mut db).initialize().unwrap();
        let key = SigningKey::from_bytes(&[74; 32]);
        let publisher = TrustedPublisher {
            publisher_id: "com.example".into(),
            key_id: "automation-key".into(),
            public_key: key.verifying_key(),
        };
        PublisherTrustRegistry::new(&mut db)
            .enroll(
                "local",
                &publisher,
                0,
                &TrustDecision {
                    decision_id: "enroll".into(),
                    authority_ref: "kernel-test".into(),
                },
                1,
            )
            .unwrap();
        let trust = PublisherTrustRegistry::new(&mut db)
            .trusted_publisher("local", "com.example", "automation-key")
            .unwrap();
        let manifest:Manifest=serde_json::from_value(json!({
            "schema":"chariox.app.v1","appId":"com.example.automation","version":"1.0.0",
            "publisher":{"id":"com.example","keyId":"automation-key","name":"Developer"},
            "sdkVersion":"0.5.0","appContractVersion":1,"minKernelProtocol":292,
            "resourcePolicy":"chariox.app.resources.v1","runtime":{"engine":"node","entry":"runtime/main.js"},
            "ui":{"entry":"ui/index.html"},"events":"schemas/events.json","capabilities":{}
        })).unwrap();
        let files=BTreeMap::from([
            ("runtime/main.js".into(),b"export default function register() {}".to_vec()),
            ("ui/index.html".into(),b"<!doctype html><title>Automation</title>".to_vec()),
            ("schemas/events.json".into(),serde_json::to_vec(&json!({"events":[{"name":"changed","schemaVersion":1,"direction":"outgoing","payloadSchema":{"type":"object","additionalProperties":false,"properties":{}}}]})).unwrap()),
        ]);
        let bytes = pack(&manifest, &files, &key, &Limits::default()).unwrap();
        let verified = verify(&bytes, &VerificationPolicy::new(292, vec![publisher])).unwrap();
        let candidate = VerifiedInstallCandidate::from_verified(&verified, &trust).unwrap();
        let mut registry = InstallationRegistry::new(&mut db);
        let token = registry
            .create_and_stage_verified("installed", "local", &candidate, 2)
            .unwrap()
            .token;
        registry
            .decide(
                &token,
                CapabilityDecision::Approved {
                    approval: CapabilityApproval {
                        decision_id: "approve".into(),
                        authority_ref: "kernel-test".into(),
                    },
                },
                3,
            )
            .unwrap();
        registry.quiesce(&token, 4).unwrap();
        registry.mark_prepared(&token, 5).unwrap();
        registry
            .commit_verified(&token, "local", &trust, 6)
            .unwrap();
        let binding = registry.staged_trust("local", &token).unwrap();
        let catalog = Arc::new(
            EventCatalog::compile(
                &verified,
                Arc::new(AppCatalog::compile(&verified, &binding, &trust).unwrap()),
            )
            .unwrap(),
        );
        Self { root, catalog }
    }
    pub(super) fn open(&self) -> DurableKernelStateStore {
        DurableKernelStateStore::open_owned(self.root.join("kernel.sqlite")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

pub(super) fn workflow() -> (SessionService, String, String) {
    use crate::{
        agent::{AgentInstance, GridPosition},
        session::CreateSessionRequest,
    };
    let mut sessions = SessionService::new(&crate::config::DaemonConfig::for_tests());
    let mut session = sessions
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    session.set_agents(vec![AgentInstance::new(
        "agent",
        "agent-ref",
        session.id(),
        None,
        "dev-stub",
        None,
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    )]);
    let id = session.id().to_owned();
    sessions.restore_session(session);
    let workflow = sessions
        .create_workflow(&id, Some("automation-test".into()))
        .unwrap();
    let node = sessions
        .add_workflow_node(&id, workflow.id(), "agent")
        .unwrap();
    let endpoint = sessions
        .create_workflow_endpoint(&id, workflow.id(), node.id(), Some("main".into()))
        .unwrap();
    let publication = sessions
        .create_workflow_publication(
            &id,
            workflow.id(),
            endpoint.id(),
            Some("default".into()),
            Some("events".into()),
            Some("event_based".into()),
            None,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "local".into(),
        )
        .unwrap();
    (sessions, id, publication.id().into())
}
