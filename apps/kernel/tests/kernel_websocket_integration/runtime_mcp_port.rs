use std::sync::Arc;

use crate::support::kernel_websocket::*;
use chariox_kernel::provider::LaunchProviderRequest;
use chariox_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use chariox_kernel::{DaemonApp, DaemonConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout, Duration};

struct RunningKernel {
    app: Arc<Mutex<DaemonApp>>,
    session_id: String,
    server: JoinHandle<Result<(), chariox_kernel::DaemonError>>,
    shutdown: oneshot::Sender<()>,
    // The provider run's working directory: removed when this drops.
    _workspace: ExecutionWorkspace,
}

/// A kernel with its own websocket port and the runtime MCP port left at the
/// default, which another process on the machine already listens on.
fn start_kernel(default_runtime_mcp_port: u16) -> RunningKernel {
    let workspace = ExecutionWorkspace::new();
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
    config.runtime_mcp_port = default_runtime_mcp_port;
    let app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let session_id = app
        .sessions_mut()
        .create_session(workspace.session_request())
        .expect("session should be created")
        .id()
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let (shutdown, shutdown_rx) = oneshot::channel::<()>();
    let server_app = Arc::clone(&app);
    let server = tokio::spawn(async move {
        run_kernel_websocket_server_on_listener(server_app, kernel_websocket_listener, async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    RunningKernel {
        app,
        session_id,
        server,
        shutdown,
        _workspace: workspace,
    }
}

async fn runtime_mcp_answers(url: &str) -> bool {
    let authority = url
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .expect("runtime MCP URL should be http://host:port/mcp");
    let Ok(mut stream) = tokio::net::TcpStream::connect(authority).await else {
        return false;
    };
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    );
    if stream.write_all(request.as_bytes()).await.is_err() {
        return false;
    }
    // A socket nobody serves accepts the connection and never answers.
    let mut response = Vec::new();
    let _ = timeout(
        Duration::from_millis(500),
        stream.read_to_end(&mut response),
    )
    .await;
    response.starts_with(b"HTTP/1.1 ")
}

/// The URL the kernel hands its providers, once this kernel (and not another
/// one on the same port) serves it.
async fn own_runtime_mcp_url(kernel: &mut RunningKernel, taken_by_others: &[String]) -> String {
    let wait = async {
        loop {
            assert!(
                !kernel.server.is_finished(),
                "the kernel stopped instead of starting: {:?}",
                (&mut kernel.server).await
            );
            let url = kernel.app.lock().await.config().runtime_mcp_url();
            if !taken_by_others.contains(&url) && runtime_mcp_answers(&url).await {
                return url;
            }
            sleep(Duration::from_millis(25)).await;
        }
    };
    timeout(Duration::from_secs(10), wait)
        .await
        .expect("the kernel should serve a runtime MCP endpoint of its own")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernels_with_the_default_runtime_mcp_port_taken_start_on_ports_of_their_own() {
    // Another kernel's listener: a socket of a different process, which this
    // process's hot-reload bookkeeping knows nothing about.
    let (default_runtime_mcp_port, _other_kernel_listener) = reserved_kernel_listener();
    let mut first = start_kernel(default_runtime_mcp_port);
    let first_url = own_runtime_mcp_url(&mut first, &[]).await;
    let mut second = start_kernel(default_runtime_mcp_port);
    let second_url = own_runtime_mcp_url(&mut second, std::slice::from_ref(&first_url)).await;

    assert_ne!(first_url, second_url);
    for (kernel, url) in [(&first, &first_url), (&second, &second_url)] {
        let run = kernel
            .app
            .lock()
            .await
            .launch_provider(LaunchProviderRequest::new(
                &kernel.session_id,
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ))
            .expect("provider run should launch");
        assert_eq!(
            run.runtime_mcp_server_url(),
            Some(url.as_str()),
            "a provider is told the port its own kernel serves"
        );
    }

    for kernel in [first, second] {
        let _ = kernel.shutdown.send(());
        let _ = timeout(Duration::from_secs(5), kernel.server).await;
    }
}
