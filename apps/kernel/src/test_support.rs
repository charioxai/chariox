use crate::account_profile::{
    ProviderAccountAuthState, ProviderAccountProfile, ProviderAccountProfileRegistry,
    ProviderAccountUsageSnapshot,
};
use crate::{DaemonApp, DaemonConfig, DaemonError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

mod environment;
#[cfg(unix)]
mod pre_exec_child;
mod runtime_mcp;
pub(crate) use environment::{environment_test_isolated, isolate_environment_test};
#[cfg(unix)]
pub(crate) use pre_exec_child::PreExecChild;
pub(crate) use runtime_mcp::TestRuntimeMcp;

/// Run ambient-environment fixtures outside the parallel test process. A mutex
/// around writers cannot protect unlocked readers or children inheriting env.
/// Place this first in any test that mutates env or calls an env-mutating fixture.
macro_rules! isolated_env_test {
    () => {
        if $crate::test_support::isolate_environment_test() {
            return;
        }
    };
}
pub(crate) use isolated_env_test;

static TEST_WORKTREE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Dropped router tasks may briefly retain the durable store's ownership fence.
/// Yield until they finish, retrying only that explicit busy-owner error.
pub(crate) async fn bootstrap_after_test_owner_exit(config: DaemonConfig) -> DaemonApp {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            match DaemonApp::bootstrap(config.clone()) {
                Ok(app) => return app,
                Err(DaemonError::LocalTransport {
                    operation: "durable_state.acquire_owner",
                    message,
                }) if message == "durable state is already owned by another kernel" => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(error) => panic!("restart should bootstrap: {error}"),
            }
        }
    })
    .await
    .expect("the dropped kernel's background owners should exit")
}

/// A real, disposable working directory for provider-launch fixtures.
///
/// Provider launch preflight intentionally rejects synthetic paths. Keep the
/// fixture's lifetime tied to the test so callers cannot accidentally leave
/// shared worktree state behind.
pub(crate) struct TestWorktree {
    path: PathBuf,
}

impl TestWorktree {
    pub(crate) fn new(label: &str) -> Self {
        let nonce = crate::session::unix_epoch_ms();
        let sequence = TEST_WORKTREE_COUNTER.fetch_add(1, Ordering::Relaxed);
        // Provider workspaces must remain outside protected runtime state, even
        // when TMPDIR points at disk-backed Chariox test scratch.
        let root = std::env::var_os("CHARIOX_TEST_WORKTREE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = root.join(format!(
            "chariox-test-worktree-{label}-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("test worktree should exist");
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn session_request(&self) -> crate::session::CreateSessionRequest {
        let path = self.path.display().to_string();
        crate::session::CreateSessionRequest::new(path.clone(), path)
    }
}

impl Drop for TestWorktree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Serves the kernel's runtime MCP endpoint on an ephemeral port for the rest
/// of the test process and points `config` at it.
///
/// A real OpenCode launch waits until its `chariox` MCP server reports
/// connected. Without a listener on the configured port, those tests only pass
/// when another test happens to serve the fixed default port. The server runs
/// on its own thread and throwaway kernel, because provider launches block the
/// caller's runtime while it holds the app lock.
pub(crate) fn serve_runtime_mcp(config: &mut DaemonConfig) {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("runtime MCP listener should bind");
    listener
        .set_nonblocking(true)
        .expect("runtime MCP listener should be non-blocking");
    config.runtime_mcp_port = listener
        .local_addr()
        .expect("runtime MCP listener should expose an address")
        .port();
    std::thread::Builder::new()
        .name("test-runtime-mcp".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            // Boot before entering the runtime: bootstrap may drive its own.
            let app = DaemonApp::bootstrap(DaemonConfig::for_tests())
                .expect("runtime MCP kernel should boot");
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime MCP test runtime should start")
                .block_on(async move {
                    let router = std::sync::Arc::new(
                        crate::runtime::router::CommandRouter::with_interactive_capacity(
                            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
                            16,
                        ),
                    );
                    let listener = tokio::net::TcpListener::from_std(listener)
                        .expect("runtime MCP listener should register");
                    let _ = crate::transport::mcp_server::run_mcp_http_server_on_listener(
                        router, listener,
                    )
                    .await;
                });
        })
        .expect("runtime MCP test server thread should spawn");
}

/// Synthetic runtime fixtures must not discover the developer's real provider credentials.
/// A fresh unavailable usage observation keeps optional native usage probes out of the fixture.
pub(crate) fn authenticate_provider_account(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<ProviderAccountProfile, DaemonError> {
    let mut usage = ProviderAccountUsageSnapshot::unavailable(profile_id, provider);
    usage.observed_at_ms = Some(crate::session::unix_epoch_ms());
    usage.source = "test.fixture".to_string();
    registry.update_observation(
        owner_user_id,
        provider,
        profile_id,
        ProviderAccountAuthState::Authenticated,
        None,
        None,
        None,
        Some(usage),
    )
}

pub(crate) fn bootstrap_authenticated_app(config: DaemonConfig) -> Result<DaemonApp, DaemonError> {
    let app = DaemonApp::bootstrap(config)?;
    let registry = app.provider_account_profile_registry();
    for profile in registry.list_all()? {
        authenticate_provider_account(
            &registry,
            &profile.owner_user_id,
            &profile.provider,
            &profile.profile_id,
        )?;
    }
    Ok(app)
}

#[test]
fn authenticated_fixture_keeps_new_unavailable_accounts_blocked() {
    let app = bootstrap_authenticated_app(DaemonConfig::for_tests()).expect("fixture should boot");
    let registry = app.provider_account_profile_registry();
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    registry
        .require_authenticated(owner, "codex", "default", Some("gpt-test"), "test")
        .expect("default fixture account should be authenticated");
    let unavailable = registry
        .create_managed(owner, "codex", "Unavailable")
        .expect("unavailable account should register");
    assert!(registry
        .require_authenticated(
            owner,
            "codex",
            &unavailable.profile_id,
            Some("gpt-test"),
            "test"
        )
        .is_err());
}

/// MP-11: exercise FIFO rejection without leaving a blocked test thread behind.
#[cfg(unix)]
pub(crate) fn assert_fifo_rejected(reader: impl FnOnce(PathBuf) -> bool + Send + 'static) {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    let root = std::env::temp_dir().join(format!("mp11-fifo-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&root).unwrap();
    let fifo = root.join("input");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let (send, receive) = std::sync::mpsc::channel();
    let input = fifo.clone();
    let thread = std::thread::spawn(move || {
        send.send(reader(input)).unwrap();
    });
    let outcome = receive.recv_timeout(std::time::Duration::from_secs(1));
    let timely = outcome.is_ok();
    let settled = if outcome.is_ok() {
        outcome
    } else {
        // Release an old blocking open/read without leaving a reader behind.
        use std::io::Write;
        let mut unblock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(&fifo)
            .unwrap();
        unblock.write_all(b"synthetic-fifo-fixture").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        drop(unblock);
        receive.recv_timeout(std::time::Duration::from_secs(2))
    };
    let joined = thread.join();
    std::fs::remove_dir_all(root).unwrap();
    joined.unwrap();
    let refused = settled.expect("released FIFO reader should finish");
    assert!(timely, "FIFO open blocked before regular-file validation");
    assert!(refused, "FIFO was admitted as regular input");
}
