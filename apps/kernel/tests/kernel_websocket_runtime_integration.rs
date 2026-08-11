mod support;

fn run_kernel_websocket_runtime_test<F>(future: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name("kernel-websocket-runtime-test".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .thread_stack_size(8 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("tokio runtime should build")
                .block_on(future);
        })
        .expect("test thread should spawn")
        .join()
        .expect("test thread should finish");
}

#[path = "kernel_websocket_runtime_integration/project_lifecycle.rs"]
mod project_lifecycle;
#[path = "kernel_websocket_runtime_integration/prompt_replay.rs"]
mod prompt_replay;
#[path = "kernel_websocket_runtime_integration/prompt_responsiveness.rs"]
mod prompt_responsiveness;
#[path = "kernel_websocket_runtime_integration/provider_launch.rs"]
mod provider_launch;
#[path = "kernel_websocket_runtime_integration/provider_processes.rs"]
mod provider_processes;
#[path = "kernel_websocket_runtime_integration/structured_io.rs"]
mod structured_io;
