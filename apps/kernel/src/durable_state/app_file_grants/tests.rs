use super::*;

struct Fixture(std::path::PathBuf, DurableKernelStateStore);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-file-grants-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        rusqlite::Connection::open(store.path())
            .unwrap()
            .execute_batch(
                "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json)
                 VALUES('docs','com.example.docs','alice',3,3,'{}')",
            )
            .unwrap();
        Self(root, store)
    }
    fn pick(&self, id: &str, multiple: bool, expires_ms: u64) -> FilePick {
        self.1
            .app_file_grant(FileGrantCommand::Create(FilePick {
                operation_id: id.into(),
                owner: "alice".into(),
                installation: "docs".into(),
                generation: 3,
                accept: vec![".md".into()],
                multiple,
                state: PickState::Pending,
                expires_ms,
                grants: Vec::new(),
            }))
            .unwrap()
            .unwrap()
    }
    fn grant(
        &self,
        owner: &str,
        id: &str,
        names: &[&str],
        now_ms: u64,
    ) -> Result<Option<FilePick>, &'static str> {
        self.1.app_file_grant(FileGrantCommand::Grant {
            owner: owner.into(),
            operation_id: id.into(),
            files: names
                .iter()
                .map(|name| GrantedFile {
                    name: (*name).into(),
                    contents: format!("# {name}").into_bytes(),
                })
                .collect(),
            now_ms,
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn only_the_owner_grants_accepted_files_once_and_each_grant_imports_once() {
    let fixture = Fixture::new();
    fixture.pick("pick-1", false, 1_000);
    assert_eq!(
        fixture.grant("mallory", "pick-1", &["a.md"], 10),
        Err("NOT_FOUND")
    );
    assert_eq!(
        fixture.grant("alice", "pick-1", &["a.md", "b.md"], 10),
        Err("INVALID_ARGUMENT")
    );
    assert_eq!(
        fixture.grant("alice", "pick-1", &["a.exe"], 10),
        Err("INVALID_ARGUMENT")
    );
    assert_eq!(
        fixture.grant("alice", "pick-1", &["../a.md"], 10),
        Err("INVALID_ARGUMENT")
    );
    let granted = fixture
        .grant("alice", "pick-1", &["Notes.MD"], 10)
        .unwrap()
        .unwrap();
    assert_eq!(granted.state, PickState::Granted);
    assert_eq!(granted.grants.len(), 1);
    assert_eq!(
        fixture.grant("alice", "pick-1", &["a.md"], 11),
        Err("CONFLICT")
    );
    let grant = &granted.grants[0];
    // Another installation cannot read or import it.
    assert_eq!(
        fixture
            .1
            .app_file_grant_contents("alice", "other", grant, 12),
        Ok(None)
    );
    let file = fixture
        .1
        .app_file_grant_contents("alice", "docs", grant, 12)
        .unwrap()
        .unwrap();
    assert_eq!(
        (file.name.as_str(), file.contents.as_slice()),
        ("Notes.MD", b"# Notes.MD".as_slice())
    );
    fixture
        .1
        .app_file_grant(FileGrantCommand::Imported {
            owner: "alice".into(),
            installation: "docs".into(),
            grant_id: grant.clone(),
        })
        .unwrap();
    assert_eq!(
        fixture
            .1
            .app_file_grant_contents("alice", "docs", grant, 13),
        Ok(None)
    );
    assert_eq!(
        fixture.1.app_file_grant(FileGrantCommand::Imported {
            owner: "alice".into(),
            installation: "docs".into(),
            grant_id: grant.clone(),
        }),
        Err("NOT_FOUND")
    );
}

#[test]
fn declined_expired_and_stale_picks_release_nothing() {
    let fixture = Fixture::new();
    fixture.pick("declined", true, 1_000);
    fixture
        .1
        .app_file_grant(FileGrantCommand::Decline {
            operation_id: "declined".into(),
            now_ms: 5,
        })
        .unwrap();
    assert_eq!(
        fixture.grant("alice", "declined", &["a.md"], 6),
        Err("CONFLICT")
    );

    fixture.pick("late", true, 100);
    assert_eq!(
        fixture.grant("alice", "late", &["a.md"], 100),
        Err("CONFLICT")
    );

    // A granted file expires unimported and its bytes are dropped.
    fixture.pick("granted", true, 1_000);
    let grant = fixture
        .grant("alice", "granted", &["a.md", "b.md"], 10)
        .unwrap()
        .unwrap()
        .grants[0]
        .clone();
    fixture
        .1
        .app_file_grant(FileGrantCommand::Expire {
            now_ms: 10 + GRANT_MS,
        })
        .unwrap();
    assert_eq!(
        fixture
            .1
            .app_file_grant_contents("alice", "docs", &grant, 10),
        Ok(None)
    );
    assert_eq!(
        fixture
            .1
            .app_file_pick("alice", "docs", "granted")
            .unwrap()
            .unwrap()
            .state,
        PickState::Expired
    );

    // An update makes the old generation's pending pick unusable.
    fixture.pick("stale", false, u64::MAX / 2);
    rusqlite::Connection::open(fixture.1.path())
        .unwrap()
        .execute(
            "UPDATE app_installations SET generation=4, allocated_generation=4",
            [],
        )
        .unwrap();
    fixture
        .1
        .app_file_grant(FileGrantCommand::Expire { now_ms: 20 })
        .unwrap();
    assert!(fixture.1.pending_app_file_picks(8).unwrap().is_empty());
}

#[test]
fn open_picks_are_bounded_per_installation() {
    let fixture = Fixture::new();
    for index in 0..MAX_OPEN {
        fixture.pick(&format!("pick-{index}"), false, 1_000);
    }
    assert_eq!(
        fixture.1.app_file_grant(FileGrantCommand::Create(FilePick {
            operation_id: "one-too-many".into(),
            owner: "alice".into(),
            installation: "docs".into(),
            generation: 3,
            accept: Vec::new(),
            multiple: false,
            state: PickState::Pending,
            expires_ms: 1_000,
            grants: Vec::new(),
        })),
        Err("LIMIT_EXCEEDED")
    );
    assert_eq!(fixture.1.pending_app_file_picks(8).unwrap().len(), 1);
}
