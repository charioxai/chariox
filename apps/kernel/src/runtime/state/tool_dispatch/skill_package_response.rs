use std::path::Path;

use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;

impl KernelRuntimeState {
    pub(super) fn apply_remote_skill_package_response(
        &self,
        workspace_root: &Path,
        home_kernel_id: &str,
        mut result: crate::transport::runtime_tools::RuntimeToolResult,
        skill_package: Option<crate::skill::CharioxSkillPackage>,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        if !result.ok {
            return Ok(result);
        }
        let Some(skill_package) = skill_package else {
            return Ok(result);
        };
        let materialized_root = crate::skill::materialize_skill_package(
            &crate::skill::remote_skill_materialization_base(workspace_root).join(home_kernel_id),
            &skill_package,
        )?;
        if let Some(skill) = result
            .payload
            .get_mut("skill")
            .and_then(serde_json::Value::as_object_mut)
        {
            skill.insert(
                "path".to_string(),
                serde_json::Value::String(
                    materialized_root
                        .join("SKILL.md")
                        .to_string_lossy()
                        .to_string(),
                ),
            );
            skill.insert(
                "materialized_root".to_string(),
                serde_json::Value::String(materialized_root.to_string_lossy().to_string()),
            );
            skill.insert(
                "version_hash".to_string(),
                serde_json::Value::String(skill_package.version_hash),
            );
            skill.insert(
                "files".to_string(),
                serde_json::Value::Array(
                    skill_package
                        .files
                        .into_iter()
                        .map(|file| serde_json::Value::String(file.path))
                        .collect(),
                ),
            );
        }
        Ok(result)
    }
}
