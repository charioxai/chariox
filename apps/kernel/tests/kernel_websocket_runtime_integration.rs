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

fn run_kernel_websocket_runtime_test_with_paused_clock<F>(future: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name("kernel-websocket-controlled-clock-test".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .start_paused(true)
                .build()
                .expect("controlled-clock runtime should build")
                .block_on(async move {
                    // A blocking task inhibits Tokio's automatic clock
                    // advancement during real socket I/O. Its channel also
                    // supplies a wall-clock missing-event guard, independent
                    // of the frozen response and initialization timers.
                    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
                    let (expired_tx, expired_rx) = tokio::sync::oneshot::channel();
                    let clock_hold = tokio::task::spawn_blocking(move || {
                        if matches!(
                            finished_rx.recv_timeout(std::time::Duration::from_secs(30)),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                        ) {
                            let _ = expired_tx.send(());
                        }
                    });
                    tokio::select! {
                        () = future => {},
                        _ = expired_rx => panic!("controlled-clock test did not finish within 30s"),
                    }
                    finished_tx.send(()).expect("the clock hold should release");
                    clock_hold.await.expect("the clock hold should join");
                });
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
