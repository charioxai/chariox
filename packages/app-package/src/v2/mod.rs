//! Package schema `chariox.app.v2`: the frozen contract that every Apps v2
//! consumer builds on (docs/APPS_V2_CONTRACTS.md §1–§2, §5).
//!
//! v1 stays readable by the v1 verifier; the two schemas never share a signing
//! subject, so a v1 signature cannot be replayed as v2 or the other way round.

mod identity;
mod manifest;
mod package;
mod ui_actions;
mod view_agent;
mod world;

#[cfg(test)]
mod tests;

pub use identity::{
    registration_outcome, AppId, Namespace, RegistrationOutcome, RegistrationRefusal,
    SignerPrincipal, DEV_PUBLISHER_ID,
};
pub use manifest::{Build, ManifestV2, RequiredFeature, RuntimeEngineV2, RuntimeV2};
pub use package::{
    pack, verify, PackageDigests, PolicyV2, ReviewEntry, UnverifiedReview, VerifiedPackageV2,
    MAX_REVIEWS, MAX_REVIEW_BYTES, REVIEW_DIR, REVIEW_SCHEMA,
};
pub use ui_actions::{
    check_agreement, check_registration, UiActionDeclaration, UiActionEffect, UiActionField,
    UiActionMismatch,
};
pub use view_agent::{
    check_input, check_registered, find_declared, plan_batch, BatchPlan, ViewActionError,
};
pub use world::{check_world, SupportedWorld, WorldId, WorldRefusal, APP_WORLD, MIGRATION_WORLD};

pub const APP_SCHEMA_V2: &str = "chariox.app.v2";
pub const APP_CONTRACT_VERSION_V2: u32 = 2;
pub const INTEGRITY_SCHEMA_V2: &str = "chariox.integrity.v2";
pub const SIGNATURE_SCHEMA_V2: &str = "chariox.package-signature.v2";
