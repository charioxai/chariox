use super::{EnrollmentError, Result, MAX_BUNDLE};
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Inventory {
    schema: String,
    target: String,
    runtime_version: String,
    worker_abi: u32,
    node_version: String,
    node_module_abi: u32,
    sdk_version: String,
    source_commit: String,
    pub files: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Entry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub executable: bool,
}
impl Inventory {
    pub(super) fn validate(&self, target: &str) -> Result<()> {
        if self.schema != "chariox.app-runtime-inventory.v1"
            || self.target != target
            || self.runtime_version != "0.1.0"
            || self.worker_abi != 1
            || self.node_version != "24.20.0"
            || self.node_module_abi != 137
            || self.sdk_version != chariox_app_package::SUPPORTED_SDK_VERSION
            || !hex(&self.source_commit, 40)
            || self.files.len() > 40
        {
            return Err(EnrollmentError::Contract);
        }
        let mut expected = expected_paths(target)?;
        let mut prior: Option<&str> = None;
        for entry in &self.files {
            if !expected.remove(entry.path.as_str())
                || prior.is_some_and(|prior| prior >= entry.path.as_str())
                || !hex(&entry.sha256, 64)
                || entry.size == 0
                || entry.size > MAX_BUNDLE
                || entry.executable != executable(&entry.path)
            {
                return Err(EnrollmentError::Contract);
            }
            prior = Some(&entry.path);
        }
        if !expected.is_empty() {
            return Err(EnrollmentError::Contract);
        }
        Ok(())
    }
}
pub(super) fn executable(path: &str) -> bool {
    matches!(
        path,
        "chariox-app-worker" | "chariox-app-domain-entry" | "chariox-bwrap"
    )
}
pub(super) fn expected_paths(target: &str) -> Result<BTreeSet<String>> {
    let lock: serde_json::Value =
        serde_json::from_str(include_str!("../../../../apps/app-worker/bundle.lock.json"))
            .map_err(|_| EnrollmentError::Contract)?;
    let mut paths: BTreeSet<String> = [
        "CHARIOX-LICENSE",
        "NODE-LICENSE",
        "runtime.lock.json",
        "bundle.lock.json",
        "native-artifact-manifest.json",
        "bundle-manifest.json",
        "chariox-app-worker",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for path in lock["bootstrap"]
        .as_array()
        .ok_or(EnrollmentError::Contract)?
    {
        paths.insert(path.as_str().ok_or(EnrollmentError::Contract)?.into());
    }
    for path in lock["sdkFiles"]
        .as_array()
        .ok_or(EnrollmentError::Contract)?
    {
        paths.insert(format!(
            "sdk/{}",
            path.as_str().ok_or(EnrollmentError::Contract)?
        ));
    }
    match target {
        "linux-x64" | "linux-arm64" => {
            paths.extend(
                [
                    "libnode.so.137",
                    "libchariox-app-runtime.so",
                    "chariox-app-domain-entry",
                    "chariox-bwrap",
                ]
                .map(str::to_owned),
            );
        }
        "darwin-x64" | "darwin-arm64" => {
            paths.extend(["libnode.137.dylib", "libchariox-app-runtime.dylib"].map(str::to_owned));
        }
        _ => return Err(EnrollmentError::Contract),
    }
    Ok(paths)
}
pub(super) fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
}
pub(super) fn target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux-x64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("macos", "x86_64") => Ok("darwin-x64"),
        ("macos", "aarch64") => Ok("darwin-arm64"),
        _ => Err(EnrollmentError::Contract),
    }
}
