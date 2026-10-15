//! MP-08 / MP-10 / MP-11: official execution-profile skill discovery.
use super::CodexClient;
use crate::error::DaemonError;
impl CodexClient {
    pub fn native_skill_candidates(
        &self,
        cwd: &std::path::Path,
        force_reload: bool,
    ) -> Result<Vec<crate::provider::skill_inventory::NativeSkillCandidate>, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let response = self.send_request::<serde_json::Value>(
            &mut socket,
            &mut 1,
            "skills/list",
            serde_json::json!({ "cwds": [cwd], "forceReload": force_reload }),
        )?;
        crate::provider::skill_inventory::codex_skill_candidates(&response, cwd)
    }
}
