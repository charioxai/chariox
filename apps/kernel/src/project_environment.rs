//! MP-08: Project environment policy shared by every kernel placement.
//! Discovery contains names and locators only. Values belong to the Vault.

mod discovery;
mod import;
mod index;
mod launch;
mod materialization_transaction;
mod materialize;
mod model;
mod private_overlay;
mod refresh;
mod resolver;
mod review;
mod store;
mod transfer;

pub use discovery::*;
pub(crate) use import::*;
pub use index::*;
pub(crate) use launch::*;
pub use materialize::*;
pub use model::*;
pub(crate) use private_overlay::*;
pub use refresh::*;
pub use resolver::*;
pub(crate) use resolver::open_workspace_file;
pub use review::*;
pub use store::*;
pub use transfer::*;

#[cfg(test)]
mod tests;
