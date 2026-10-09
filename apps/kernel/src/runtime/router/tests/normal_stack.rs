use super::*;

#[tokio::test]
async fn normal_dispatch_construction_fits_default_thread_stack() {
    crate::test_support::isolated_env_test!();
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let request = LocalDaemonRequest::GetTerminalCommandCatalog(
        crate::local::GetTerminalCommandCatalogRequest {},
    );
    let command = remote_command_for_request(&request, Some("alice"));
    // CI enlarges libtest stacks; explicitly retain Rust's ordinary 2 MiB here.
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            // Even this tiny arm previously reserved every unrelated handler's
            // future temporaries and overflowed before the future was polled.
            drop(router.dispatch_normal_or_background(command, request));
        })
        .unwrap()
        .join()
        .unwrap();
}
