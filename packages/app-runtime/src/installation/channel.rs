//! Dev versus update channels (docs/APPS_V2_CONTRACTS.md §3): which installation
//! an App ID resolves to, and which releases may share one version.
use serde::{Deserialize, Serialize};

/// An installation is `dev` (the developer's own working copy, signed by the
/// kernel's dev identity) or `release` (the registered or distributed App).
/// The two are separate installations of one App ID, with separate data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationChannel {
    Dev,
    Release,
}

/// An optional subscription of a release installation. Changing it changes
/// which releases are offered, never the installation, its data or its trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    Stable,
    Preview,
}

/// Where the bytes came from. Provenance only: every source reaches the same
/// staging coordinator (§4 of the contract) and none writes installation rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionSource {
    #[default]
    LocalFile,
    DevPush,
    Register,
    Library,
    DirectFeed,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChannelError {
    #[error("a dev installation belongs to exactly one Room")]
    DevNeedsRoom,
    #[error("only a dev installation is bound to a Room")]
    ReleaseHasRoom,
    #[error("a dev installation has no update subscription")]
    DevHasUpdateChannel,
}

/// What selection sees of one installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationRef {
    pub installation_id: String,
    pub owner_id: String,
    pub app_id: String,
    pub channel: InstallationChannel,
    /// The Room whose automatic dev bindings use this installation.
    pub dev_room_id: Option<String>,
    pub update_channel: Option<UpdateChannel>,
}

impl InstallationRef {
    pub fn validate(&self) -> Result<(), ChannelError> {
        match (self.channel, &self.dev_room_id, self.update_channel) {
            (InstallationChannel::Dev, None, _) => Err(ChannelError::DevNeedsRoom),
            (InstallationChannel::Dev, _, Some(_)) => Err(ChannelError::DevHasUpdateChannel),
            (InstallationChannel::Release, Some(_), _) => Err(ChannelError::ReleaseHasRoom),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    pub owner_id: String,
    pub app_id: String,
    /// The Room the caller works in, when the caller has one.
    pub room_id: Option<String>,
    /// `None` is automatic: the dev copy only in its own Room, else release.
    pub channel: Option<InstallationChannel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Resolution {
    Found {
        installation_id: String,
    },
    NotFound,
    /// Several installations fit. The caller must name one; the first match is
    /// never taken.
    Ambiguous {
        installation_ids: Vec<String>,
    },
}

pub fn resolve(installations: &[InstallationRef], selector: &Selector) -> Resolution {
    let of_channel =
        |channel: InstallationChannel, room: Option<&String>| -> Vec<&InstallationRef> {
            installations
                .iter()
                .filter(|item| {
                    item.owner_id == selector.owner_id
                        && item.app_id == selector.app_id
                        && item.channel == channel
                        && room.is_none_or(|room| item.dev_room_id.as_ref() == Some(room))
                })
                .collect()
        };
    let found = |matches: Vec<&InstallationRef>| match matches.as_slice() {
        [] => Resolution::NotFound,
        [only] => Resolution::Found {
            installation_id: only.installation_id.clone(),
        },
        many => Resolution::Ambiguous {
            installation_ids: many
                .iter()
                .map(|item| item.installation_id.clone())
                .collect(),
        },
    };
    match selector.channel {
        Some(InstallationChannel::Release) => found(of_channel(InstallationChannel::Release, None)),
        Some(InstallationChannel::Dev) => found(of_channel(
            InstallationChannel::Dev,
            selector.room_id.as_ref(),
        )),
        None => {
            let dev = selector
                .room_id
                .as_ref()
                .map(|room| of_channel(InstallationChannel::Dev, Some(room)))
                .unwrap_or_default();
            if dev.is_empty() {
                found(of_channel(InstallationChannel::Release, None))
            } else {
                found(dev)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{app_id} {version} is already released with different content; publish a new version")]
pub struct VersionConflict {
    pub app_id: String,
    pub version: String,
}

/// `(publisher, App ID, version)` names immutable content. `known_digest` is
/// the package digest already recorded for it; dev saves are identified by
/// their digest alone and skip this check.
pub fn check_immutable_version(
    app_id: &str,
    version: &str,
    known_digest: Option<&str>,
    incoming_digest: &str,
) -> Result<(), VersionConflict> {
    match known_digest {
        Some(known) if known != incoming_digest => Err(VersionConflict {
            app_id: app_id.to_owned(),
            version: version.to_owned(),
        }),
        _ => Ok(()),
    }
}
