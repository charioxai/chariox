//! Installed privileged App storage helper. Never started by a development test.
#[cfg(all(not(test), not(debug_assertions)))]
const _: () = chariox_app_runtime::assert_production_build();

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = chariox_app_runtime::worker_process::run_app_storage_helper(
        std::env::args().skip(1).collect(),
    ) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("app_storage_platform_unsupported");
    std::process::exit(1);
}
