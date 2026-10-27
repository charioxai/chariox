use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
pub use chariox_app_runtime::{
    app_catalog::AppCatalog,
    app_inbox::{self, InboxRoute},
    app_outbox::{self, AppOutbox, EventCatalog, Invocation, Occurrence},
    installation::{
        CapabilityApproval, CapabilityDecision, InstallationRegistry, StageTrustBinding,
        VerifiedInstallCandidate,
    },
    managed_state::{
        self, ManagedStateStore, StateChanges, StateScope, StateWrite, Wake, WakeChange,
    },
    publisher_trust::{PublisherTrustRegistry, TrustDecision, TrustedPublisherSnapshot},
    release_store::{ReleaseStore, StageBudget},
};
use ed25519_dalek::SigningKey;
pub use rusqlite::Connection;
pub use serde_json::json;
use std::{
    collections::BTreeMap,
    io::Write,
    os::unix::{fs::PermissionsExt, process::ExitStatusExt},
    process::{Child, Command},
    sync::Arc,
    time::{Duration, Instant},
};
pub use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn case(name: &str) -> PathBuf {
    let parent = PathBuf::from(std::env::var_os("CHARIOX_FAULT_ROOT").unwrap());
    assert!(parent.is_absolute() && parent.is_dir());
    let root = parent.join(name);
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    root
}
pub fn cleanup(root: &Path) {
    fn writable(path: &Path) {
        if fs::symlink_metadata(path).unwrap().is_dir() {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            for entry in fs::read_dir(path).unwrap() {
                writable(&entry.unwrap().path());
            }
        }
    }
    writable(root);
    fs::remove_dir_all(root).unwrap();
}
pub fn open(root: &Path) -> Connection {
    let mut db = Connection::open(root.join("kernel.sqlite")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .unwrap();
    InstallationRegistry::new(&mut db).initialize().unwrap();
    PublisherTrustRegistry::new(&mut db).initialize().unwrap();
    ManagedStateStore::new(&mut db).initialize().unwrap();
    AppOutbox::initialize(&db).unwrap();
    app_inbox::initialize(&db).unwrap();
    db
}
pub struct Package {
    pub bytes: Vec<u8>,
    pub publisher: TrustedPublisher,
}
impl Package {
    pub fn new(version: u32, schema: u32) -> Self {
        let seed: [u8; 32] = fs::read(std::env::var_os("CHARIOX_FAULT_KEY").unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let key = SigningKey::from_bytes(&seed);
        let mut manifest: Manifest = serde_json::from_value(json!({
            "schema":"chariox.app.v1","appId":"com.example.b5","version":format!("{version}.0.0"),
            "publisher":{"id":"com.example.b5","keyId":"throwaway","name":"B5 component drill"},
            "sdkVersion":chariox_app_package::SUPPORTED_SDK_VERSION,"appContractVersion":1,"minKernelProtocol":367,
            "resourcePolicy":"chariox.app.resources.v1","runtime":{"engine":"node","entry":"runtime/main.js"},
            "ui":{"entry":"ui/index.html"},"events":"schemas/events.json","capabilities":{}
        })).unwrap();
        let payload = json!({"type":"object","additionalProperties":false,"required":["text"],"properties":{"text":{"type":"string"}}});
        let mut files = BTreeMap::from([
            ("runtime/main.js".into(), b"export default function register() {}".to_vec()),
            ("ui/index.html".into(), b"<!doctype html><title>B5</title>".to_vec()),
            ("schemas/events.json".into(), serde_json::to_vec(&json!({"events":[
                {"name":"changed","schemaVersion":1,"direction":"outgoing","payloadSchema":payload},
                {"name":"received","schemaVersion":1,"direction":"incoming","payloadSchema":payload}
            ]})).unwrap())
        ]);
        if schema > 0 {
            assert_eq!(schema, 1);
            manifest.migrations = Some(serde_json::from_value(json!({"directory":"migrations","targetVersion":1,"steps":[{"from":0,"to":1,"entry":"migrations/001.js"}]})).unwrap());
            files.insert(
                "migrations/001.js".into(),
                b"export default async function migrate() {}".to_vec(),
            );
        }
        let bytes = pack(&manifest, &files, &key, &Limits::default()).unwrap();
        Self {
            bytes,
            publisher: TrustedPublisher {
                publisher_id: "com.example.b5".into(),
                key_id: "throwaway".into(),
                public_key: key.verifying_key(),
            },
        }
    }
    pub fn candidate(&self, db: &mut Connection) -> VerifiedInstallCandidate {
        let trust = trust(db);
        let verified = verify(
            &self.bytes,
            &VerificationPolicy::new(367, vec![self.publisher.clone()]),
        )
        .unwrap();
        VerifiedInstallCandidate::from_verified(&verified, &trust).unwrap()
    }
    pub fn catalog(&self, db: &mut Connection) -> Arc<EventCatalog> {
        let trust = trust(db);
        let binding = InstallationRegistry::new(db)
            .journal("installed")
            .unwrap()
            .into_iter()
            .find(|row| row.token.generation == 1)
            .unwrap()
            .token;
        let binding = InstallationRegistry::new(db)
            .staged_trust("owner", &binding)
            .unwrap();
        let verified = verify(
            &self.bytes,
            &VerificationPolicy::new(367, vec![self.publisher.clone()]),
        )
        .unwrap();
        Arc::new(
            EventCatalog::compile(
                &verified,
                Arc::new(AppCatalog::compile(&verified, &binding, &trust).unwrap()),
            )
            .unwrap(),
        )
    }
    pub fn extract(
        &self,
        root: &Path,
        at: impl FnMut(
            chariox_app_runtime::release_store::StageCheckpoint,
        ) -> Result<(), chariox_app_runtime::release_store::ReleaseStoreError>,
    ) {
        let store = ReleaseStore::open_or_create(&root.join("kernel.sqlite")).unwrap();
        let verified = verify(
            &self.bytes,
            &VerificationPolicy::new(367, vec![self.publisher.clone()]),
        )
        .unwrap();
        store
            .stage_with_checkpoint(&verified, &self.bytes, budget(), at)
            .unwrap();
    }
}
pub fn budget() -> StageBudget {
    StageBudget {
        max_stage_bytes: 8 * 1024 * 1024,
        reserved_bytes: 8 * 1024 * 1024,
        host_reserve_bytes: 0,
    }
}
pub fn trust(db: &mut Connection) -> TrustedPublisherSnapshot {
    PublisherTrustRegistry::new(db)
        .trusted_publisher("owner", "com.example.b5", "throwaway")
        .unwrap()
}
pub fn approval() -> CapabilityApproval {
    CapabilityApproval {
        decision_id: "component-policy-fixture".into(),
        authority_ref: "component-test-not-human-health".into(),
    }
}
pub fn scope(generation: u64) -> StateScope<'static> {
    StateScope::new("owner", "installed", generation).unwrap()
}
pub fn changes(schema: u32, text: &str) -> StateChanges {
    StateChanges::new(
        schema,
        vec![],
        vec![StateWrite::Put {
            key: "saved".into(),
            value: json!({"text":text}),
        }],
    )
    .unwrap()
}
pub fn route() -> InboxRoute {
    InboxRoute {
        route_id: "route".into(),
        owner_id: "owner".into(),
        installation_id: "installed".into(),
        event_name: "received".into(),
        source_event_type: "test.b5/message".into(),
        source_event_version: 1,
        active: true,
        source: None,
    }
}
pub fn occurrence() -> Occurrence {
    Occurrence {
        automation_id: "automation".into(),
        occurrence_id: app_outbox::occurrence_id("b5-event", 100).unwrap(),
        event_version: 1,
        occurred_at_ms: 100,
        schedule_revision: None,
        payload: json!({"text":"new"}),
        invocation: Invocation {
            prompt: "Component no-effect fixture".into(),
            artifacts: vec![],
        },
    }
}
pub fn seed(db: &mut Connection) {
    let package = Package::new(1, 0);
    PublisherTrustRegistry::new(db)
        .enroll(
            "owner",
            &package.publisher,
            0,
            &TrustDecision {
                decision_id: "throwaway-local-test".into(),
                authority_ref: "component-test".into(),
            },
            1,
        )
        .unwrap();
    let candidate = package.candidate(db);
    let token = InstallationRegistry::new(db)
        .create_and_stage_verified("installed", "owner", &candidate, 2)
        .unwrap()
        .token;
    let trust = trust(db);
    let mut registry = InstallationRegistry::new(db);
    registry
        .decide(
            &token,
            CapabilityDecision::Approved {
                approval: approval(),
            },
            3,
        )
        .unwrap();
    registry.quiesce(&token, 4).unwrap();
    registry.mark_prepared(&token, 5).unwrap();
    registry
        .commit_verified(&token, "owner", &trust, 6)
        .unwrap();
    ManagedStateStore::new(db)
        .transaction(scope(1), &changes(0, "acknowledged"))
        .unwrap();
    app_inbox::create_route_in(db, &route(), 1).unwrap();
    let catalog = package.catalog(db);
    db.execute("INSERT INTO app_automations (owner_id,installation_id,automation_id,revision,event_name,event_version,schema_digest,session_id,publication_id,endpoint_id,queue_id,status) VALUES ('owner','installed','automation',1,'changed',1,?1,'session','publication','endpoint','default','active')",[catalog.schema_digest("changed").unwrap()]).unwrap();
}
pub fn stop(root: &Path, receipt: serde_json::Value) -> ! {
    let mut file = fs::File::create(root.join("checkpoint.json")).unwrap();
    file.write_all(serde_json::to_string(&receipt).unwrap().as_bytes())
        .unwrap();
    file.sync_all().unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
pub fn kill_at(root: &Path, point: &str) {
    let mut child = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "crash_child", "--nocapture"])
            .env("CHARIOX_FAULT_CHILD_ROOT", root)
            .env("CHARIOX_FAULT_POINT", point)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while !root.join("checkpoint.json").exists()
        || fs::metadata(root.join("checkpoint.json")).unwrap().len() == 0
    {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before {point}"
        );
        assert!(Instant::now() < deadline, "checkpoint deadline: {point}");
        std::thread::sleep(Duration::from_millis(10));
    }
    // Marker must parse before cancellation; a partial write is not a receipt.
    loop {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(
            &fs::read(root.join("checkpoint.json")).unwrap(),
        ) {
            println!(
                "{}",
                json!({"case":point,"phase":"checkpoint","receipt":value})
            );
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    child.0.kill().unwrap();
    assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
}
pub fn receipt(root: &Path, phase: &str, db: &mut Connection) {
    let installed = InstallationRegistry::new(db).get("installed").unwrap();
    let value: String = db.query_row("SELECT value_json FROM app_state_values WHERE installation_id='installed' AND key='saved'",[],|row|row.get(0)).unwrap();
    let count = |table: &str| {
        db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
    };
    println!(
        "{}",
        json!({"case":root.file_name().unwrap().to_str().unwrap(),"phase":phase,"active":installed.active,"pending":installed.pending_generation,"admission_paused":installed.admission_paused,"state":serde_json::from_str::<serde_json::Value>(&value).unwrap(),"inbox":count("app_inbox"),"outbox":count("app_outbox"),"wakes":count("app_wakes"),"migration_snapshots":count("app_state_snapshot_values")})
    );
}
