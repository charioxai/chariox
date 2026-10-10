//! MP-08: Project environment policy shared by every kernel placement.
//! Discovery contains names and locators only. Values belong to the Vault.

mod aggregate;
mod aggregate_projection;
mod aggregate_store;
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
mod refresh;
mod resolver;
mod review;
mod revisions;
mod scope_conflicts;
mod specification_validation;
mod store;
mod transfer;

pub use aggregate::*;
pub use aggregate_projection::*;
pub use detect::*;
pub use detect_store::*;
pub use discovery::*;
pub(crate) use import::*;
pub use index::*;
pub(crate) use launch::*;
pub(crate) use manual_files::verify_manual_files;
pub use materialize::*;
pub use model::*;
pub(crate) use private_overlay::*;
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
