use std::fmt;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{ErrorCode, PackageError, Result};

pub use chariox_app_wit::{APP_WORLD, MIGRATION_WORLD};

/// `<namespace>:<package>/<world>@<major>.<minor>.<patch>`, the exact world a
/// manifest names. Pre-release and build metadata are not part of the contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldId {
    name: String,
    version: Version,
}

impl WorldId {
    pub fn parse(value: &str) -> Result<Self> {
        let invalid = || {
            PackageError::new(
                ErrorCode::InvalidManifest,
                "world must look like chariox:app/app@1.0.0",
            )
        };
        let (name, version) = value.split_once('@').ok_or_else(invalid)?;
        let (namespace, rest) = name.split_once(':').ok_or_else(invalid)?;
        let (package, world) = rest.split_once('/').ok_or_else(invalid)?;
        let label = |part: &str| {
            part.len() <= 64
                && part.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        };
        let version = Version::parse(version).map_err(|_| invalid())?;
        if ![namespace, package, world].into_iter().all(label)
            || !version.pre.is_empty()
            || !version.build.is_empty()
        {
            return Err(invalid());
        }
        Ok(Self {
            name: name.to_owned(),
            version,
        })
    }

    /// `chariox:app/app`, the identity that versions of one world share.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &Version {
        &self.version
    }
}

impl fmt::Display for WorldId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}

/// One world generation a kernel links: every minor up to `minor` of `major`.
/// A kernel inside a deprecation window lists two majors of one world.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportedWorld {
    pub name: String,
    pub major: u64,
    pub minor: u64,
}

/// Why a world is refused before any component code runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "refusal", rename_all = "snake_case")]
pub enum WorldRefusal {
    UnknownWorld,
    UnsupportedMajor { supported_majors: Vec<u64> },
    MinorTooNew { kernel_minor: u64 },
}

impl WorldRefusal {
    pub(crate) fn into_error(self, requested: &WorldId) -> PackageError {
        let message = match self {
            Self::UnknownWorld => format!("world {requested} is not linked by this kernel"),
            Self::UnsupportedMajor { supported_majors } => format!(
                "world {requested} is not supported; this kernel links major {supported_majors:?}"
            ),
            Self::MinorTooNew { kernel_minor } => format!(
                "world {requested} is newer than this kernel's minor {kernel_minor}; update the kernel"
            ),
        };
        PackageError::new(ErrorCode::IncompatibleContract, message)
    }
}

/// Minor versions only add `@since` items, so an older minor always links. A
/// patch never changes the contract.
pub fn check_world(
    requested: &WorldId,
    supported: &[SupportedWorld],
) -> std::result::Result<(), WorldRefusal> {
    let same_world: Vec<_> = supported
        .iter()
        .filter(|world| world.name == requested.name)
        .collect();
    if same_world.is_empty() {
        return Err(WorldRefusal::UnknownWorld);
    }
    let Some(world) = same_world
        .iter()
        .find(|world| world.major == requested.version.major)
    else {
        let mut supported_majors: Vec<u64> = same_world.iter().map(|world| world.major).collect();
        supported_majors.sort_unstable();
        return Err(WorldRefusal::UnsupportedMajor { supported_majors });
    };
    if requested.version.minor > world.minor {
        return Err(WorldRefusal::MinorTooNew {
            kernel_minor: world.minor,
        });
    }
    Ok(())
}
