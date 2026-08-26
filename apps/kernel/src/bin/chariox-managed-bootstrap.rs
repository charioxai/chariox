fn main() -> Result<(), chariox_kernel::DaemonError> {
    if let Ok(log_path) = chariox_kernel::logging::init_process_logger("managed-bootstrap") {
        chariox_kernel::logging::info_with_fields(
            "managed_bootstrap.start",
            "managed kernel bootstrap supervisor starting",
            serde_json::json!({ "log_path": log_path.display().to_string() }),
        );
    }
    chariox_kernel::managed_bootstrap::run_from_env()
}
