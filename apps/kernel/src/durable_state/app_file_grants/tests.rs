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
    let claim = |installation: &str, now_ms: u64| {
        fixture.1.claim_app_file_grant(FileGrantCommand::Claim {
            owner: "alice".into(),
            installation: installation.into(),
            grant_id: grant.clone(),
            now_ms,
        })
    };
    let settle = |release: bool| {
        let (owner, installation, grant_id) =
            ("alice".to_owned(), "docs".to_owned(), grant.clone());
        fixture.1.app_file_grant(if release {
            FileGrantCommand::Release {
                owner,
                installation,
                grant_id,
            }
        } else {
            FileGrantCommand::Imported {
                owner,
                installation,
                grant_id,
            }
        })
    };
    // Another installation cannot take it.
    assert_eq!(claim("other", 12), Err("NOT_FOUND"));
    let file = claim("docs", 12).unwrap();
    assert_eq!(
        (file.name.as_str(), file.contents.as_slice()),
        ("Notes.MD", b"# Notes.MD".as_slice())
    );
    // A concurrent import cannot claim it too; a failed one gives it back.
    assert_eq!(claim("docs", 12), Err("NOT_FOUND"));
    settle(true).unwrap();
    claim("docs", 13).unwrap();
    settle(false).unwrap();
    assert_eq!(claim("docs", 14), Err("NOT_FOUND"));
    // Released after publishing, the dropped bytes stay dropped.
    settle(true).unwrap();
    assert_eq!(claim("docs", 15), Err("NOT_FOUND"));
}

#[test]
fn an_update_keeps_picks_and_grants_and_an_uninstall_ends_them() {
    let fixture = Fixture::new();
    fixture.pick("granted", false, 1_000);
    let grant = fixture
        .grant("alice", "granted", &["a.md"], 10)
        .unwrap()
        .unwrap()
        .grants[0]
        .clone();
    fixture.pick("pending", false, u64::MAX / 2);
    let database = rusqlite::Connection::open(fixture.1.path()).unwrap();
    // The update to generation 4 commits.
    database
        .execute(
            "UPDATE app_installations SET generation=4, allocated_generation=4",
            [],
        )
        .unwrap();
    fixture
        .1
        .app_file_grant(FileGrantCommand::Expire { now_ms: 20 })
        .unwrap();
    // The unanswered pick is still shown to the owner, and the new release
    // imports the grant chosen for the old one.
    assert_eq!(fixture.1.pending_app_file_picks(8).unwrap().len(), 1);
    let claim = |grant_id: &str| {
        fixture.1.claim_app_file_grant(FileGrantCommand::Claim {
            owner: "alice".into(),
            installation: "docs".into(),
            grant_id: grant_id.into(),
            now_ms: 21,
        })
    };
    assert_eq!(claim(&grant).unwrap().name, "a.md");
    fixture
        .1
        .app_file_grant(FileGrantCommand::Release {
            owner: "alice".into(),
            installation: "docs".into(),
            grant_id: grant.clone(),
        })
        .unwrap();
    // Uninstalled: nothing more is shown, and the bytes are dropped.
    database
        .execute("UPDATE app_installations SET active_json=NULL", [])
        .unwrap();
    fixture
        .1
        .app_file_grant(FileGrantCommand::Expire { now_ms: 22 })
        .unwrap();
    assert!(fixture.1.pending_app_file_picks(8).unwrap().is_empty());
    assert_eq!(claim(&grant), Err("NOT_FOUND"));
    assert_eq!(
        fixture
            .1
            .app_file_pick("alice", "docs", "granted")
            .unwrap()
            .unwrap()
            .state,
        PickState::Expired
    );
}

#[test]
fn declined_and_expired_picks_release_nothing() {
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
        fixture.1.claim_app_file_grant(FileGrantCommand::Claim {
            owner: "alice".into(),
            installation: "docs".into(),
            grant_id: grant.clone(),
            now_ms: 10,
        }),
        Err("NOT_FOUND")
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

#[test]
fn one_answer_stays_within_a_local_request_frame() {
    let fixture = Fixture::new();
    fixture.pick("big", true, 1_000);
    let file = |name: &str| GrantedFile {
        name: name.into(),
        contents: vec![b'x'; 400 * 1024],
    };
    assert_eq!(
        fixture.1.app_file_grant(FileGrantCommand::Grant {
            owner: "alice".into(),
            operation_id: "big".into(),
            files: vec![file("a.md"), file("b.md")],
            now_ms: 10,
        }),
        Err("INVALID_ARGUMENT")
    );
}

#[test]
fn the_owner_revokes_unanswered_picks_and_unimported_grants() {
    let fixture = Fixture::new();
    let revoke = |owner: &str, installation: &str, operation: Option<&str>| {
        fixture.1.revoke_app_file_grants(FileGrantCommand::Revoke {
            owner: owner.into(),
            installation: installation.into(),
            operation_id: operation.map(Into::into),
            now_ms: 20,
        })
    };
    let claim = |grant_id: &str| {
        fixture.1.claim_app_file_grant(FileGrantCommand::Claim {
            owner: "alice".into(),
            installation: "docs".into(),
            generation: 3,
            grant_id: grant_id.into(),
            now_ms: 12,
        })
    };
    let settle = |grant_id: &str, imported: bool| {
        let (owner, installation, grant_id) =
            ("alice".to_owned(), "docs".to_owned(), grant_id.to_owned());
        fixture.1.app_file_grant(if imported {
            FileGrantCommand::Imported {
                owner,
                installation,
                grant_id,
            }
        } else {
            FileGrantCommand::Release {
                owner,
                installation,
                grant_id,
            }
        })
    };
    fixture.pick("granted", true, 1_000);
    let granted = fixture
        .grant("alice", "granted", &["a.md", "b.md"], 10)
        .unwrap()
        .unwrap()
        .grants;
    // a.md is imported before the revoke and stays imported.
    claim(&granted[0]).unwrap();
    settle(&granted[0], true).unwrap();
    // c.md is being imported while the owner revokes.
    fixture.pick("importing", false, 1_000);
    let importing = fixture
        .grant("alice", "importing", &["c.md"], 10)
        .unwrap()
        .unwrap()
        .grants;
    claim(&importing[0]).unwrap();
    fixture.pick("pending", false, 1_000);
    fixture.pick("other", false, 1_000);

    // Only the owner, and only for that installation's own picks.
    assert_eq!(revoke("mallory", "docs", Some("pending")), Err("NOT_FOUND"));
    assert_eq!(revoke("alice", "other", Some("pending")), Err("NOT_FOUND"));
    assert_eq!(revoke("alice", "docs", Some("missing")), Err("NOT_FOUND"));
    // One pick.
    assert_eq!(
        revoke("alice", "docs", Some("other")),
        Ok(RevokedFiles {
            pending: vec!["other".into()],
            requests: 1,
            files: 0,
        })
    );
    // Everything else: b.md was not imported; c.md is in flight.
    assert_eq!(
        revoke("alice", "docs", None),
        Ok(RevokedFiles {
            pending: vec!["pending".into()],
            requests: 3,
            files: 1,
        })
    );
    assert_eq!(claim(&granted[1]), Err("NOT_FOUND"));
    // The in-flight import failed after the revoke: nothing comes back.
    settle(&importing[0], false).unwrap();
    assert_eq!(claim(&importing[0]), Err("NOT_FOUND"));
    assert!(fixture.1.pending_app_file_picks(8).unwrap().is_empty());
    for operation in ["granted", "importing", "pending", "other"] {
        assert_eq!(
            fixture
                .1
                .app_file_pick("alice", "docs", operation)
                .unwrap()
                .unwrap()
                .state,
            PickState::Expired,
            "{operation}"
        );
    }
    // Nothing is left to revoke; an ended pick can be named again.
    assert_eq!(revoke("alice", "docs", None), Ok(RevokedFiles::default()));
    assert_eq!(
        revoke("alice", "docs", Some("granted")),
        Ok(RevokedFiles::default())
    );
}
