//! Private readonly code-view journal. A digest selects an installer-enrolled
//! source; neither the socket nor this record accepts arbitrary mount paths.
use super::{
    model::{self, Identity},
    Error, Result,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Kind {
    Package,
    Runtime,
}
impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::Runtime => "runtime",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct View {
    pub kind: Kind,
    pub source: Identity,
    pub underlying: Identity,
    pub mount_id: Option<u64>,
    pub sealed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub package_digest: String,
    pub runtime_digest: String,
    pub runtime_revision: u64,
    pub views: [View; 2],
}
impl Record {
    pub fn validate(&self) -> Result<()> {
        validate_pins(
            &self.package_digest,
            &self.runtime_digest,
            self.runtime_revision,
        )?;
        for (index, view) in self.views.iter().enumerate() {
            if view.kind != [Kind::Package, Kind::Runtime][index]
                || view.source.inode == 0
                || view.underlying.inode == 0
                || view.source == view.underlying
                || view.mount_id == Some(0)
                || (view.sealed && view.mount_id.is_none())
            {
                return Err(Error::Identity);
            }
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Grant {
    pub package_digest: String,
    pub runtime_digest: String,
    pub runtime_revision: u64,
    pub package_root: Identity,
    pub runtime_root: Identity,
}
pub(super) fn validate_pins(package: &str, runtime: &str, revision: u64) -> Result<()> {
    model::hex(package.strip_prefix("sha256:").ok_or(Error::Invalid)?, 64)?;
    model::hex(runtime, 64)?;
    if revision == 0 || revision > i64::MAX as u64 {
        return Err(Error::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn journal_rejects_swapped_roots_or_claimed_seal_without_mount_identity() {
        let view = |kind, inode| View {
            kind,
            source: Identity { device: 1, inode },
            underlying: Identity {
                device: 1,
                inode: inode + 10,
            },
            mount_id: None,
            sealed: false,
        };
        let record = Record {
            package_digest: format!("sha256:{}", "a".repeat(64)),
            runtime_digest: "b".repeat(64),
            runtime_revision: 1,
            views: [view(Kind::Package, 1), view(Kind::Runtime, 2)],
        };
        assert!(record.validate().is_ok());
        let mut swapped = record.clone();
        swapped.views.swap(0, 1);
        assert!(swapped.validate().is_err());
        let mut forged = record.clone();
        forged.views[0].sealed = true;
        assert!(forged.validate().is_err());
        forged.views[0].mount_id = Some(1);
        assert!(forged.validate().is_ok());
        forged.views[0].underlying = forged.views[0].source.clone();
        assert!(forged.validate().is_err());
        assert!(validate_pins(&record.package_digest, &record.runtime_digest, 0).is_err());
        assert!(validate_pins(&record.package_digest, &record.runtime_digest, u64::MAX).is_err());
    }
}
