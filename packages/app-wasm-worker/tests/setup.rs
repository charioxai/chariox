//! MP-08 / MP-10 / MP-11: Setup's signed mode must admit a user-owned worker.
use chariox_app_runtime::worker_process::wasm::WasmRuntime;
use std::{fs, os::unix::fs::MetadataExt, path::PathBuf, process::Command};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn setup_installed_user_owned_worker_opens_through_runtime() {
    let root = std::env::temp_dir().join(format!(
        "chariox-setup-worker-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let scratch = Scratch(fs::canonicalize(root).unwrap());
    let output = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/setup/install.mjs"
        ))
        .arg(&scratch.0)
        .arg(env!("CARGO_BIN_EXE_chariox-app-wasm-worker"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Setup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let worker = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    assert_eq!(fs::metadata(&worker).unwrap().uid(), unsafe {
        libc::geteuid()
    });
    let runtime = WasmRuntime::open(&worker).expect("Setup's installed worker must be trusted");
    assert!(runtime.identity().worker_build.starts_with("sha256:"));
    assert_eq!(fs::metadata(worker).unwrap().mode() & 0o777, 0o555);
}
