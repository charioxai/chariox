use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub mod catalog;
#[cfg(test)]
mod catalog_tests;
pub mod frontmatter;
mod home_journal;
pub(crate) mod leased_composer;
pub mod leased_names;
mod load_history;
pub(crate) mod model_metadata;
mod package;
#[cfg(test)]
mod package_tests;
mod paths;
mod provider_import;
mod repository;
#[cfg(test)]
mod repository_tests;
mod repository_update;
pub mod resolver;
pub mod review;
pub mod selection;
pub mod service;
pub(crate) mod source_access;
pub mod storage;
#[cfg(test)]
mod tests;
pub mod transfer;
pub(crate) mod turn_authority;
pub mod v2;
pub(crate) mod watch;

pub(crate) use package::package_skill_directory;
pub use package::{CharioxSkillPackage, CharioxSkillPackageFile};
pub use provider_import::{
    discover_provider_skill_import_candidates, ProviderSkillImportCandidate,
    ProviderSkillImportDiscovery, SkillImportSkip,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharioxSkillMetadata {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_description: Option<String>,
    pub path: PathBuf,
}
