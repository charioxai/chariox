//! A Project's skills as the Skills section of its Environment: the Project-scope records of the home
//! catalog, read as packages for an Import and installed through the ordinary prepare and publish path.
use super::{
    catalog::failure,
    package::{materialize_skill_package_for_id, package_skill_directory},
    service::SkillCatalogService,
    v2::*,
    CharioxSkillPackage,
};
use crate::error::DaemonError;
use std::path::Path;

impl SkillCatalogService {
    /// The names of the Project-scope skills of `project_id`, in catalog order.
    pub(crate) fn project_skill_names(&self, project_id: &str) -> Result<Vec<String>, DaemonError> {
        let snapshot = self.storage.snapshot()?;
        Ok(snapshot
            .catalog(&project_scope(project_id))
            .map(|catalog| {
                catalog
                    .skills
                    .iter()
                    .map(|skill| skill.name.clone())
                    .collect()
            })
            .unwrap_or_default())
    }

    /// The installed package of the Project-scope skill `name`, or `None` when the Project has none.
    pub(crate) fn project_skill_package(
        &self,
        project_id: &str,
        name: &str,
    ) -> Result<Option<CharioxSkillPackage>, DaemonError> {
        let snapshot = self.storage.snapshot()?;
        let Some(record) = snapshot
            .catalog(&project_scope(project_id))
            .and_then(|catalog| catalog.skills.iter().find(|skill| skill.name == name))
        else {
            return Ok(None);
        };
        package_skill_directory(&self.verified_package_root(record)?).map(Some)
    }

    /// Installs `package` as an Auto Project-scope skill unless the Project already holds its name;
    /// `false` means the held skill is left exactly as it is.
    pub(crate) fn install_project_skill_if_absent(
        &self,
        project_id: &str,
        package: &CharioxSkillPackage,
    ) -> Result<bool, DaemonError> {
        let snapshot = self.storage.snapshot()?;
        let scope = project_scope(project_id);
        if snapshot.catalog(&scope).is_some_and(|catalog| {
            catalog
                .skills
                .iter()
                .any(|skill| skill.name == package.metadata.name)
        }) {
            return Ok(false);
        }
        let staging = self.storage.directory(
            &Path::new("state/skill-staging")
                .join(format!("environment-{:032x}", rand::random::<u128>())),
        )?;
        let installed = (|| {
            let folder =
                materialize_skill_package_for_id(&staging, &package.metadata.name, package)?;
            let mutation = SkillCatalogMutation {
                scope,
                expected_revision: snapshot.revision,
                actions: vec![SkillCatalogAction::Install {
                    source: SkillInstallSource::Folder {
                        path: folder
                            .to_str()
                            .ok_or_else(|| failure("skill staging path is not UTF-8"))?
                            .to_string(),
                    },
                    collision: SkillCollisionAction::Reject,
                    package_id: None,
                }],
            };
            let prepared = self.prepare(&mutation)?;
            self.publish_authorized(&mutation, &prepared)
        })();
        let _ = std::fs::remove_dir_all(&staging);
        installed.map(|_| true)
    }
}

fn project_scope(project_id: &str) -> SkillScope {
    SkillScope::Project {
        project_id: project_id.to_string(),
    }
}
