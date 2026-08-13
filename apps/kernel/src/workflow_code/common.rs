use super::*;

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn workflow_code_definition_sha256_hex(definition: &WorkflowCodeDefinition) -> String {
    let bytes = serde_json::to_vec(definition).unwrap_or_default();
    sha256_hex(&bytes)
}

pub(super) fn chariox_home() -> Option<PathBuf> {
    std::env::var_os("CHARIOX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".chariox")))
}

pub(super) fn io_error(
    operation: &'static str,
) -> impl FnOnce(std::io::Error) -> crate::DaemonError + Copy {
    move |error| crate::DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    }
}
