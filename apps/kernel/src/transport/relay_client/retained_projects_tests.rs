//! A copy of several Projects whose registration could not finish keeps every
//! registered Project's files across later attempts and restarts, through the
//! encrypted peer import (the bridge's real entry point). Cloud admission is
//! loopback mocked.
use super::owner_context_drill::{enrolled_profile, target_router, Cleanup, EncryptedPeer};
use super::tests::{ManagedPeerRequestHarness, ScopedEnv};
use super::*;
use crate::config::DaemonConfig;
use crate::managed_context::outbound::*;
use crate::managed_context::outbound_service::{
    ManagedContextTransferTarget, ManagedContextTransferTicket,
};
use crate::managed_context::owner_authority::TICKET_ENDPOINT;
use crate::managed_context::owner_managed::*;
use crate::managed_context::{
    development::{retention_faults, *},
    kernel::*,
    package::*,
};
use crate::runtime::terminal_pairings::public_key_thumbprint;
use chariox_relay::auth::RelaySubjectKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const OWNER: &str = "owner-user";
const PROJECT_FOLDERS: [&str; 2] = ["project", "project-two"];

struct Fixture {
    root: PathBuf,
    source: DaemonConfig,
    target: DaemonConfig,
    identity: RelayCallerIdentity,
    request: ManagedContextOutboundTransferRequest,
    state_db: PathBuf,
    server: tokio::task::JoinHandle<()>,
    _env: Vec<ScopedEnv>,
    _cleanup: Cleanup,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.email=tests@chariox.local",
            "-c",
            "user.name=Chariox Tests",
        ])
        .args(args)
        .current_dir(path)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_text(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("run git");
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn init_repository(path: &Path, contents: &str) {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "-b", "main"]);
    std::fs::write(path.join("README.md"), contents).unwrap();
    git(path, &["add", "README.md"]);
    git(path, &["commit", "-m", "initial"]);
}

fn commit_work(repository: &Path) {
    std::fs::write(repository.join("work.txt"), b"unpushed work\n").unwrap();
    git(repository, &["add", "work.txt"]);
    git(repository, &["commit", "-m", "work in the copied Project"]);
}

/// The target's durable state fails the write that registers the second Project
/// and take any Project back, so the first Project stays registered.
fn fail_second_registration_and_every_rollback(state_db: &Path) {
    rusqlite::Connection::open(state_db)
        .expect("failure injector")
        .execute_batch(
            "CREATE TRIGGER fail_second_create BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'project.created'
               AND EXISTS (SELECT 1 FROM durable_state_events WHERE kind = 'project.created')
             BEGIN SELECT RAISE(FAIL, 'injected project create failure'); END;
             CREATE TRIGGER fail_delete BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'project.deleted'
             BEGIN SELECT RAISE(FAIL, 'injected project delete failure'); END;",
        )
        .expect("install failure");
}

fn recover_storage(state_db: &Path) {
    rusqlite::Connection::open(state_db)
        .expect("failure injector")
        .execute_batch("DROP TRIGGER fail_second_create; DROP TRIGGER fail_delete;")
        .expect("recover storage");
}

/// A restart-free stand-in for the kernel failing while it writes or lifts a
/// retention record; the flags are reset when the test ends, whatever its result.
struct RetentionFaults;

impl RetentionFaults {
    fn set(write: bool, release: bool) -> Self {
        retention_faults::FAIL_WRITE.store(write, Ordering::SeqCst);
        retention_faults::FAIL_RELEASE.store(release, Ordering::SeqCst);
        retention_faults::REJECT_COMPLETION.store(false, Ordering::SeqCst);
        Self
    }

    fn rejecting_completion() -> Self {
        let faults = Self::set(false, false);
        retention_faults::REJECT_COMPLETION.store(true, Ordering::SeqCst);
        faults
    }

    fn clear(&self) {
        retention_faults::FAIL_WRITE.store(false, Ordering::SeqCst);
        retention_faults::FAIL_RELEASE.store(false, Ordering::SeqCst);
        retention_faults::REJECT_COMPLETION.store(false, Ordering::SeqCst);
    }
}

impl Drop for RetentionFaults {
    fn drop(&mut self) {
        self.clear();
    }
}

fn fail_every_registration(state_db: &Path) {
    rusqlite::Connection::open(state_db)
        .expect("failure injector")
        .execute_batch(
            "CREATE TRIGGER fail_every_create BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'project.created'
             BEGIN SELECT RAISE(FAIL, 'injected project create failure'); END;",
        )
        .expect("install failure");
}

fn fail_every_removal(state_db: &Path) {
    rusqlite::Connection::open(state_db)
        .expect("failure injector")
        .execute_batch(
            "CREATE TRIGGER fail_every_delete BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'project.deleted'
             BEGIN SELECT RAISE(FAIL, 'injected project delete failure'); END;",
        )
        .expect("install failure");
}

fn recover_removal(state_db: &Path) {
    rusqlite::Connection::open(state_db)
        .expect("failure injector")
        .execute_batch("DROP TRIGGER fail_every_delete;")
        .expect("recover storage");
}

fn recover_registration(state_db: &Path) {
    rusqlite::Connection::open(state_db)
        .expect("failure injector")
        .execute_batch("DROP TRIGGER fail_every_create;")
        .expect("recover storage");
}

/// Retention records of the copy's publications still on disk.
fn retention_records(root: &Path) -> Vec<PathBuf> {
    fn walk(directory: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_name() == ".chariox-retained-by-project" {
                found.push(path);
            } else if entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry.file_name() != ".git"
            {
                walk(&path, found);
            }
        }
    }
    let mut found = Vec::new();
    walk(&root.join("target-home"), &mut found);
    found
}

async fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "chariox-retained-projects-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir_all(root.join("target-home/.chariox")).unwrap();
    let cleanup = Cleanup(root.clone());
    let mut env = Vec::new();
    for (name, path) in [
        ("HOME", "target-home"),
        ("CHARIOX_HOME", "target-home/.chariox"),
        ("CODEX_HOME", "target-home/.codex"),
        ("CLAUDE_CONFIG_DIR", "target-home/.claude"),
        ("OPENCODE_CONFIG_DIR", "target-home/.opencode"),
        ("XDG_CONFIG_HOME", "target-home/.config"),
        ("CHARIOX_CAPABILITY_ISOLATION_ROOT", "source-capabilities"),
    ] {
        env.push(ScopedEnv::set(name, root.join(path).as_os_str()));
    }
    env.push(ScopedEnv::set(
        "CHARIOX_MANAGED_PROVIDER_TOPOLOGY",
        std::ffi::OsStr::new(""),
    ));
    std::fs::create_dir_all(root.join("source-capabilities/user")).unwrap();
    std::fs::write(
        root.join("source-capabilities/user/user-rules.md"),
        "Read the project instructions.\n",
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let mut source = DaemonConfig::for_tests();
    source.cloud_relay = Some(enrolled_profile(&source, &api_url));
    let mut target =
        DaemonConfig::for_tests().with_session_history_root(root.join("target-history"));
    target.daemon_id = "owner-target-kernel".into();
    target.host_machine_id = "owner-target-machine".into();
    target.user_config_path = root.join("target-config.toml");
    std::fs::write(&target.user_config_path, b"# target configuration\n").unwrap();
    // A secret database needs a folder of its own: the agents' confinement hides that folder.
    target.user_config.history.operational.path = Some(
        root.join("target-operational/operational.db")
            .display()
            .to_string(),
    );
    target.user_config.artifacts.operational.root =
        Some(root.join("target-artifacts").display().to_string());
    target.user_config.artifacts.operational.index_path =
        Some(root.join("target-artifacts.db").display().to_string());
    let state_db = root.join("target-home/.chariox/state.db");
    target.user_config.state.path = Some(state_db.display().to_string());
    target.cloud_relay = Some(enrolled_profile(&target, &api_url));
    let projects = [
        ("owner-project", "project"),
        ("owner-project-two", "project-two"),
    ];
    for folder in PROJECT_FOLDERS {
        init_repository(&root.join(folder), &format!("{folder} content\n"));
    }
    let selection = OwnerManagedTransfer {
        target: ManagedContextTransferTarget {
            relay_realm_id: "owner-realm".into(),
            machine_id: target.host_machine_id.clone(),
            kernel_id: target.daemon_id.clone(),
            relay_public_key: target.relay_public_key.clone(),
            key_thumbprint: public_key_thumbprint(&target.relay_public_key),
        },
        context_selection: OwnerManagedContextSelection {
            event_connections: Default::default(),
            kernel_context: OwnerManagedKernelSelection::SourceKernelWithoutCredentials,
            development_setup: OwnerManagedDevelopmentSelection::SourceProjects {
                projects: projects
                    .into_iter()
                    .map(|(project_id, folder)| OwnerManagedSourceProject {
                        project_id: project_id.into(),
                        repositories: vec![OwnerManagedRepositorySelection {
                            role: DevelopmentRepositoryRole::Primary,
                            workspace_id: root.join(folder).display().to_string(),
                            worktree_id: None,
                        }],
                    })
                    .collect(),
            },
        },
    };
    let context_plan =
        crate::managed_bootstrap::ManagedKernelContextPlan::for_owner_managed(&source, &selection)
            .unwrap();
    let plan = context_plan.package_binding();
    let ticket = ManagedContextTransferTicket {
        environment_id: String::new(),
        context_plan,
        target: selection.target.clone(),
    };
    let source_peer = serde_json::json!({ "relayRealmId": "owner-realm", "machineId": source.host_machine_id,
        "kernelId": source.daemon_id, "relayPublicKey": source.relay_public_key, "keyThumbprint": public_key_thumbprint(&source.relay_public_key) });
    let ticket_response = serde_json::to_vec(&serde_json::json!({ "kind": "owner_managed_machine", "sourceTargetId": "directory-source",
        "source": source_peer, "target": ticket.target, "contextSelection": serde_json::to_value(&selection.context_selection).unwrap() })).unwrap();
    let peers_response = serde_json::to_vec(
        &serde_json::json!({ "peers": [{ "targetId": "directory-source", "peer": source_peer }] }),
    )
    .unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let header = loop {
                let mut chunk = [0u8; 4096];
                let read = stream.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
                if let Some(header) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..header]).to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= header + 4 + length {
                        break header;
                    }
                }
            };
            let request_headers = String::from_utf8_lossy(&bytes[..header]).into_owned();
            let response =
                if request_headers.starts_with("GET /v1/owner-managed-context-tickets/peers ") {
                    &peers_response
                } else {
                    assert!(request_headers.starts_with(&format!("POST {TICKET_ENDPOINT} ")));
                    &ticket_response
                };
            let headers = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len());
            stream.write_all(headers.as_bytes()).await.unwrap();
            stream.write_all(response).await.unwrap();
        }
    });
    let mut development_archives = Vec::new();
    for (project_id, folder) in projects {
        let development = export_development_context(DevelopmentContextExportRequest {
            project_id: project_id.into(),
            repositories: vec![DevelopmentRepositorySelection {
                workspace_id: root.join(folder).display().to_string(),
                worktree_id: None,
                worktree_path: root.join(folder),
                role: DevelopmentRepositoryRole::Primary,
            }],
            archive_path: root.join(format!("{folder}.tar.gz")),
        })
        .unwrap();
        development_archives.push(ManagedContextPackageDevelopmentArchive {
            archive_path: development.archive_path,
            archive_sha256: development.archive_sha256,
        });
    }
    let snapshot = export_kernel_context_without_credentials(KernelContextExportRequest {
        context_id: plan.context_id.clone(),
        source_kernel_id: source.daemon_id.clone(),
        source_key_thumbprint: public_key_thumbprint(&source.relay_public_key),
        target_kernel_id: target.daemon_id.clone(),
        target_key_thumbprint: selection.target.key_thumbprint.clone(),
        vault: None,
    })
    .unwrap();
    let package = export_managed_context_package(ManagedContextPackageExportRequest {
        plan: plan.clone(),
        target_environment_id: String::new(),
        source_kernel_id: source.daemon_id.clone(),
        source_key_thumbprint: public_key_thumbprint(&source.relay_public_key),
        target_kernel_id: target.daemon_id.clone(),
        target_key_thumbprint: selection.target.key_thumbprint.clone(),
        development: ManagedContextPackageDevelopment::FromSource {
            archives: development_archives,
        },
        kernel_context: ManagedContextPackageKernel::FromKernel(Box::new(snapshot)),
        provider_accounts: ManagedContextPackageProviderAccounts::None,
        git_credentials: ManagedContextPackageGitCredentials::None,
        event_connections: Default::default(),
        package_path: root.join("package.ctx"),
    })
    .unwrap();
    env.remove(6);
    // The real relay's ordinary-kernel peer projection.
    let identity = RelayCallerIdentity {
        realm_id: "owner-realm".into(),
        subject: source.host_machine_id.clone(),
        subject_kind: RelaySubjectKind::Machine,
        expires_at_ms: u64::MAX,
        token_id: Some("synthetic-owner-token-id".into()),
        user_id: Some(OWNER.into()),
        public_key_thumbprint: Some(public_key_thumbprint(&source.relay_public_key)),
    };
    let request = ManagedContextOutboundTransferRequest {
        plan,
        target_environment_id: String::new(),
        target_kernel_id: target.daemon_id.clone(),
        target_key_thumbprint: selection.target.key_thumbprint.clone(),
        package,
        capability: random_managed_context_capability(),
    };
    Fixture {
        root,
        source,
        target,
        identity,
        request,
        state_db,
        server,
        _env: env,
        _cleanup: cleanup,
    }
}

/// One attempt of the person's import, from the source kernel through the
/// encrypted peer into the target's bridge.
async fn import_attempt(router: &Arc<CommandRouter>, fixture: &Fixture) -> Result<(), DaemonError> {
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let peer = EncryptedPeer {
        harness: ManagedPeerRequestHarness {
            router,
            state: &state,
        },
        source: &fixture.source,
        identity: fixture.identity.clone(),
        target_public_key: fixture.target.relay_public_key.clone(),
        disconnect_after_chunk: AtomicBool::new(false),
        publication_interrupted: AtomicBool::new(false),
        interrupt_import: None,
    };
    transfer_managed_context_package(&peer, fixture.request.clone(), |_| {})
        .await
        .map(|_| ())
}

async fn restart(router: Arc<CommandRouter>, fixture: &Fixture) -> Arc<CommandRouter> {
    drop(router);
    target_router(fixture.target.clone()).await.0
}

async fn registered_projects(router: &CommandRouter) -> Vec<crate::session::RuntimeProject> {
    let mut projects = router.runtime_state().list_projects(OWNER, true).await;
    projects.sort_by(|left, right| left.id().cmp(right.id()));
    projects
}

/// Every Project record points at files that exist: nothing is orphaned.
fn assert_no_orphaned_records(projects: &[crate::session::RuntimeProject]) {
    for project in projects {
        for workspace in project.workspace_ids() {
            assert!(
                Path::new(workspace).join(".git").is_dir(),
                "Project `{}` is registered but its repository `{workspace}` is gone",
                project.id()
            );
        }
    }
}

/// The Project a failed registration left behind: the first of the copy.
fn the_only_registered_project(
    projects: &[crate::session::RuntimeProject],
) -> &crate::session::RuntimeProject {
    assert_eq!(projects.len(), 1, "only the first Project stays registered");
    assert!(projects[0].id().ends_with("-0"), "{}", projects[0].id());
    &projects[0]
}

fn assert_retryable(error: &DaemonError) {
    assert!(
        matches!(
            error,
            DaemonError::ManagedContext {
                retryable: true,
                ..
            }
        ),
        "{error:?}"
    );
}

fn run_on_kernel_thread<F: std::future::Future<Output = ()> + 'static>(
    drill: impl FnOnce() -> F + Send + 'static,
) {
    std::thread::Builder::new()
        .name("retained-projects-drill".into())
        .stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(drill());
        })
        .unwrap()
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

#[test]
fn a_retained_project_keeps_its_new_commits_through_every_retry() {
    crate::test_support::isolated_env_test!();
    run_on_kernel_thread(|| async {
        let _lock = crate::env_lock::lock();
        let fixture = fixture().await;
        let (router, _store) = target_router(fixture.target.clone()).await;
        fail_second_registration_and_every_rollback(&fixture.state_db);
        let error = import_attempt(&router, &fixture).await.unwrap_err();
        assert_retryable(&error);
        let router = restart(router, &fixture).await;
        let projects = registered_projects(&router).await;
        let first_id = the_only_registered_project(&projects).id().to_string();
        assert_no_orphaned_records(&projects);
        let first = PathBuf::from(&projects[0].workspace_ids()[0]);

        // The copied repository does not inherit the fixture source’s Git identity.
        git(&first, &["config", "user.email", "tests@chariox.local"]);
        git(&first, &["config", "user.name", "Chariox Tests"]);
        // The person works in the exposed Project before retrying.
        commit_work(&first);
        let work_head = git_text(&first, &["rev-parse", "HEAD"]);
        let error = import_attempt(&router, &fixture).await.unwrap_err();
        assert_retryable(&error);
        let router = restart(router, &fixture).await;
        let projects = registered_projects(&router).await;
        assert_eq!(the_only_registered_project(&projects).id(), first_id);
        assert_no_orphaned_records(&projects);
        assert_eq!(git_text(&first, &["rev-parse", "HEAD"]), work_head);
        assert!(first.join("work.txt").is_file());

        recover_storage(&fixture.state_db);
        import_attempt(&router, &fixture)
            .await
            .expect("the retry completes once storage works");
        let projects = registered_projects(&router).await;
        assert_eq!(projects.len(), 2);
        assert_no_orphaned_records(&projects);
        assert_eq!(git_text(&first, &["rev-parse", "HEAD"]), work_head);
        assert!(first.join("work.txt").is_file());
    });
}

#[test]
fn an_archived_retained_project_keeps_its_files_when_the_retry_cannot_register() {
    crate::test_support::isolated_env_test!();
    run_on_kernel_thread(|| async {
        let _lock = crate::env_lock::lock();
        let fixture = fixture().await;
        let (router, _store) = target_router(fixture.target.clone()).await;
        fail_second_registration_and_every_rollback(&fixture.state_db);
        let error = import_attempt(&router, &fixture).await.unwrap_err();
        assert_retryable(&error);
        let router = restart(router, &fixture).await;
        let projects = registered_projects(&router).await;
        let first_id = the_only_registered_project(&projects).id().to_string();
        let first = PathBuf::from(&projects[0].workspace_ids()[0]);
        let head = git_text(&first, &["rev-parse", "HEAD"]);

        let mut caller = crate::runtime::command::KernelCaller::default()
            .with_connection_class(crate::local::KernelConnectionClass::Terminal);
        caller.user_id = Some(OWNER.into());
        let request =
            crate::local::LocalDaemonRequest::ArchiveProject(crate::local::ArchiveProjectRequest {
                project_id: first_id.clone(),
            });
        let command = crate::runtime::command::KernelCommand::from_local_request_with_caller(
            "archive-retained-project",
            crate::runtime::command::KernelCommandSource::LocalCli,
            caller,
            None,
            None,
            &request,
        );
        router
            .dispatch(command, request)
            .await
            .expect("the person archives the retained Project");

        recover_storage(&fixture.state_db);
        import_attempt(&router, &fixture)
            .await
            .expect_err("an archived Project conflicts with the copy");
        let router = restart(router, &fixture).await;
        let projects = registered_projects(&router).await;
        assert_eq!(the_only_registered_project(&projects).id(), first_id);
        assert_eq!(
            projects[0].status(),
            crate::session::RuntimeProjectStatus::Archived
        );
        assert_no_orphaned_records(&projects);
        assert_eq!(git_text(&first, &["rev-parse", "HEAD"]), head);
        assert_eq!(
            std::fs::read(first.join("README.md")).unwrap(),
            b"project content\n"
        );
    });
}

#[test]
fn a_retention_that_cannot_be_written_publishes_no_project() {
    crate::test_support::isolated_env_test!();
    run_on_kernel_thread(|| async {
        let _lock = crate::env_lock::lock();
        let fixture = fixture().await;
        let (router, _store) = target_router(fixture.target.clone()).await;
        let faults = RetentionFaults::set(true, false);
        import_attempt(&router, &fixture)
            .await
            .expect_err("the copy fails before any Project is published");
        let router = restart(router, &fixture).await;
        assert!(
            registered_projects(&router).await.is_empty(),
            "a Project was published without its retention"
        );

        faults.clear();
        import_attempt(&router, &fixture)
            .await
            .expect("the retry completes once retention can be written");
        let projects = registered_projects(&router).await;
        assert_eq!(projects.len(), 2);
        assert_no_orphaned_records(&projects);
    });
}

#[test]
fn work_in_a_published_project_survives_a_retention_that_failed_after_registration() {
    crate::test_support::isolated_env_test!();
    run_on_kernel_thread(|| async {
        let _lock = crate::env_lock::lock();
        let fixture = fixture().await;
        let (router, _store) = target_router(fixture.target.clone()).await;
        fail_second_registration_and_every_rollback(&fixture.state_db);
        let faults = RetentionFaults::set(true, false);
        import_attempt(&router, &fixture)
            .await
            .expect_err("the copy fails");
        let router = restart(router, &fixture).await;

        // Whatever the first attempt published, the person works in it before retrying.
        let mut work = Vec::new();
        for project in registered_projects(&router).await {
            let repository = PathBuf::from(&project.workspace_ids()[0]);
            commit_work(&repository);
            work.push((
                repository.clone(),
                git_text(&repository, &["rev-parse", "HEAD"]),
            ));
        }
        faults.clear();
        let _ = import_attempt(&router, &fixture).await;
        let router = restart(router, &fixture).await;
        assert_no_orphaned_records(&registered_projects(&router).await);
        for (repository, head) in &work {
            assert_eq!(&git_text(repository, &["rev-parse", "HEAD"]), head);
        }

        recover_storage(&fixture.state_db);
        import_attempt(&router, &fixture)
            .await
            .expect("the retry completes once storage works");
        let projects = registered_projects(&router).await;
        assert_eq!(projects.len(), 2);
        assert_no_orphaned_records(&projects);
        for (repository, head) in &work {
            assert_eq!(&git_text(repository, &["rev-parse", "HEAD"]), head);
            assert!(repository.join("work.txt").is_file());
        }
    });
}

#[test]
fn a_copy_whose_retention_cannot_be_lifted_keeps_its_files_for_the_retry() {
    crate::test_support::isolated_env_test!();
    run_on_kernel_thread(|| async {
        let _lock = crate::env_lock::lock();
        let fixture = fixture().await;
        let (router, _store) = target_router(fixture.target.clone()).await;
        fail_every_registration(&fixture.state_db);
        let faults = RetentionFaults::set(false, true);
        let error = import_attempt(&router, &fixture).await.unwrap_err();
        assert_retryable(&error);
        let router = restart(router, &fixture).await;
        assert!(registered_projects(&router).await.is_empty());
        assert_eq!(
            retention_records(&fixture.root).len(),
            2,
            "the copied files went while their retention could not be lifted"
        );

        faults.clear();
        recover_registration(&fixture.state_db);
        import_attempt(&router, &fixture)
            .await
            .expect("the retry registers both Projects");
        let projects = registered_projects(&router).await;
        assert_eq!(projects.len(), 2);
        assert_no_orphaned_records(&projects);
    });
}

#[test]
fn a_rejected_completion_takes_back_the_copy_it_registered() {
    crate::test_support::isolated_env_test!();
    run_on_kernel_thread(|| async {
        let _lock = crate::env_lock::lock();
        let fixture = fixture().await;
        let (router, _store) = target_router(fixture.target.clone()).await;
        let _faults = RetentionFaults::rejecting_completion();
        import_attempt(&router, &fixture)
            .await
            .expect_err("Cloud rejects the completion");
        let router = restart(router, &fixture).await;
        assert!(registered_projects(&router).await.is_empty());
        assert!(
            retention_records(&fixture.root).is_empty(),
            "a rejected copy left its retention behind"
        );
    });
}

#[test]
fn a_rejected_completion_keeps_the_files_of_a_project_it_cannot_take_back() {
    crate::test_support::isolated_env_test!();
    run_on_kernel_thread(|| async {
        let _lock = crate::env_lock::lock();
        let fixture = fixture().await;
        let (router, _store) = target_router(fixture.target.clone()).await;
        fail_every_removal(&fixture.state_db);
        let faults = RetentionFaults::rejecting_completion();
        import_attempt(&router, &fixture)
            .await
            .expect_err("Cloud rejects the completion");
        let router = restart(router, &fixture).await;
        let projects = registered_projects(&router).await;
        assert_eq!(projects.len(), 2, "the Projects are still registered");
        assert_no_orphaned_records(&projects);

        faults.clear();
        recover_removal(&fixture.state_db);
        import_attempt(&router, &fixture)
            .await
            .expect("the retry completes");
        let projects = registered_projects(&router).await;
        assert_eq!(projects.len(), 2);
        assert_no_orphaned_records(&projects);
    });
}
