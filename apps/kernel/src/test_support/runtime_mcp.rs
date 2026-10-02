use crate::{DaemonApp, DaemonConfig};

/// A real MCP listener on an OS-assigned port, owned by the fixture that needs
/// it. Provider launch may block its caller, so serve on a separate runtime.
pub(crate) struct TestRuntimeMcp {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TestRuntimeMcp {
    pub(crate) fn serve(config: &mut DaemonConfig) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        config.runtime_mcp_port = listener.local_addr().unwrap().port();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("test-runtime-mcp".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(move || {
                let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async move {
                        let router = std::sync::Arc::new(
                            crate::runtime::router::CommandRouter::with_interactive_capacity(
                                std::sync::Arc::new(tokio::sync::Mutex::new(app)), 16,
                            ),
                        );
                        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                        tokio::select! {
                            result = crate::transport::mcp_server::run_mcp_http_server_on_listener(router, listener) => {
                                result.expect("fixture MCP server should serve");
                            }
                            _ = stopped => {}
                        }
                    });
            })
            .unwrap();
        Self {
            stop: Some(stop),
            thread: Some(thread),
        }
    }
}

impl Drop for TestRuntimeMcp {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().expect("fixture MCP server should stop");
        }
    }
}
