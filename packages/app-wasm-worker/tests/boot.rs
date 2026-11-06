//! MP-08 / MP-09 / MP-10 / MP-11: the authenticated boot release must carry a worker
//! that `WasmRuntime::open` trusts for the service-user kernel that boots from it.
use chariox_app_runtime::worker_process::wasm::WasmRuntime;
use std::{fs, os::unix::fs::MetadataExt, path::PathBuf, process::Command};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn boot_installed_worker_opens_through_runtime() {
    // The boot installer only trusts a root-owned image key, as on a real allocation.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("skipped: the boot installer requires root");
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "chariox-boot-worker-{}-{}",
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
            "/tests/boot/install.mjs"
        ))
        .arg(&scratch.0)
        .arg(env!("CARGO_BIN_EXE_chariox-app-wasm-worker"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "boot install failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let worker = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    let runtime = WasmRuntime::open(&worker).expect("the boot-installed worker must be trusted");
    assert!(runtime.identity().worker_build.starts_with("sha256:"));
    assert_eq!(fs::metadata(worker).unwrap().mode() & 0o777, 0o555);
}
