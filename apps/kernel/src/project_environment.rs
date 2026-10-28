//! MP-08: Project environment policy shared by every kernel placement.
//! Discovery contains names and locators only. Values belong to the Vault.

mod aggregate;
mod aggregate_projection;
mod aggregate_store;
mod check;
mod check_admission;
mod check_assets;
pub(crate) mod check_executable;
mod check_files;
mod check_gui_identity;
mod check_health;
pub(crate) mod check_inventory;
mod check_journal;
mod check_package_inventory;
mod check_platform;
mod check_runner;
mod check_start;
mod check_store;
mod check_tools;
mod dependency_graph;
mod detect;
mod detect_importers;
mod detect_index;
mod detect_metadata;
mod detect_model;
mod detect_store;
mod discovery;
mod import;
mod index;
mod launch;
mod manual_files;
mod materialization_transaction;
mod materialize;
mod model;
mod private_overlay;
mod read_cache;
mod read_view;
mod refresh;
mod resolver;
mod review;
mod revisions;
mod scope_conflicts;
mod software_constraints;
mod specification_validation;
mod store;
mod transfer;

pub use aggregate::*;
pub use aggregate_projection::*;
pub(crate) use check::*;
#[doc(hidden)]
pub use check_files::run_files_measurement_worker;
pub(crate) use check_start::*;
pub(crate) use check_store::*;
pub use detect::*;
pub use detect_metadata::project_environment_detect_prompt;
pub use detect_store::*;
pub use discovery::*;
pub(crate) use import::*;
pub use index::*;
pub(crate) use launch::*;
pub(crate) use manual_files::verify_manual_files;
pub use materialize::*;
pub use model::*;
pub(crate) use private_overlay::*;
pub use read_view::*;
pub use refresh::*;
pub(crate) use resolver::open_workspace_file;
pub use resolver::*;
pub use review::*;
pub use revisions::*;
pub use store::*;
pub use transfer::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod aggregate_tests;

#[cfg(test)]
mod detect_tests;

#[cfg(test)]
mod check_tests;

#[cfg(test)]
mod revision_exclusion_validation_tests;

#[cfg(test)]
mod check_interpreter_trust_tests;
#[cfg(test)]
mod revision_service_reference_tests;

#[cfg(test)]
mod check_journal_tests;

#[cfg(test)]
#[path = "project_environment/review_environment_tests.rs"]
mod review_environment_tests;
