mod contract;
mod discovery;
mod policy;

#[cfg(test)]
mod tests;

pub use contract::{
    EnvironmentContract, EnvironmentIdentity, LockfileDigest, RepositoryManifest,
    TargetKernelRealm, ToolchainInput, ToolchainKind, ToolchainSpec, UtilityKind, UtilitySpec,
    ValidationProbe, ValidationProbeKind,
};
pub use discovery::{discover, discover_repository_environment};
pub use policy::{
    ProjectEnvironmentError, CONTRACT_SCHEMA_VERSION, MANIFEST_RELATIVE_PATH,
    MANIFEST_SCHEMA_VERSION, MAX_LOCKFILE_BYTES, MAX_MANIFEST_BYTES, MAX_MANIFEST_ENTRIES,
    MAX_PATH_BYTES, MAX_STRING_BYTES, POLICY_VERSION,
};
