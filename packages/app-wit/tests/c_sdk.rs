//! A real C compile against the installed, checksum-pinned SDK toolchain.
//! Run explicitly: cargo test -p chariox-app-wit --test c_sdk -- --ignored
use chariox_app_wit::{check_component, APP_WORLD};
use std::{path::PathBuf, process::Command, time::SystemTime};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "requires Python 3.12+ and CHARIOX_C_TOOLCHAIN installed by app-sdk-c"]
fn source_built_c_template_passes_admission() {
    let sdk = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../app-sdk-c");
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "chariox-c-admission-{}-{unique}",
        std::process::id()
    )));
    std::fs::create_dir(&scratch.0).unwrap();
    let output = scratch.0.join("template.wasm");
    assert!(Command::new("bash")
        .arg(sdk.join("build.sh"))
        .arg(&output)
        .status()
        .expect("start C template build")
        .success());
    let bytes = std::fs::read(output).unwrap();
    check_component(&bytes, APP_WORLD)
        .expect("source-built C template must use the real admission policy");
}
