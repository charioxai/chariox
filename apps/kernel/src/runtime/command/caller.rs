use serde::{Deserialize, Serialize};

use chariox_relay::auth::RelaySubjectKind;
use chariox_relay::protocol::RelayCallerIdentity;

use crate::session::DEFAULT_LOCAL_USER_ID;

use super::KernelCommand;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelCommandSource {
    LocalCli,
    LocalIpc,
    RelayClient,
    RelayPeer,
    DaemonBackground,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelCallerKind {
    LocalClient,
    RemoteClient,
    RemoteKernel,
    HostedService,
    Metaagent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelCaller {
    pub caller_id: String,
    pub caller_kind: KernelCallerKind,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub realm_id: Option<String>,
    #[serde(default)]
    pub public_key_thumbprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metaagent_id: Option<String>,
}

impl Default for KernelCaller {
    fn default() -> Self {
        Self::for_source(&KernelCommandSource::LocalCli)
    }
}

impl KernelCaller {
    pub fn for_source(source: &KernelCommandSource) -> Self {
        let (caller_id, caller_kind) = match source {
            KernelCommandSource::LocalCli => ("local-cli", KernelCallerKind::LocalClient),
            KernelCommandSource::LocalIpc => ("local-ipc", KernelCallerKind::LocalClient),
            KernelCommandSource::RelayClient => {
                ("relay-client-unverified", KernelCallerKind::RemoteClient)
            }
            KernelCommandSource::RelayPeer => {
                ("relay-peer-unverified", KernelCallerKind::RemoteKernel)
            }
            KernelCommandSource::DaemonBackground => {
                ("daemon-background", KernelCallerKind::LocalClient)
            }
        };
        Self {
            caller_id: caller_id.to_string(),
            caller_kind,
            user_id: None,
            client_id: None,
            machine_id: None,
            realm_id: None,
            public_key_thumbprint: None,
            metaagent_id: None,
        }
    }

    pub fn from_relay_identity(identity: RelayCallerIdentity) -> Self {
        let (caller_kind, client_id, machine_id) = match identity.subject_kind {
            RelaySubjectKind::Client => (
                KernelCallerKind::RemoteClient,
                Some(identity.subject.clone()),
                None,
            ),
            RelaySubjectKind::Kernel | RelaySubjectKind::Machine => (
                KernelCallerKind::RemoteKernel,
                None,
                Some(identity.subject.clone()),
            ),
            RelaySubjectKind::Service => (KernelCallerKind::HostedService, None, None),
        };
        Self {
            caller_id: identity.subject,
            caller_kind,
            user_id: identity.user_id,
            client_id,
            machine_id,
            realm_id: Some(identity.realm_id),
            public_key_thumbprint: identity.public_key_thumbprint,
            metaagent_id: None,
        }
    }
}

pub(crate) fn command_caller_user_id(command: &KernelCommand) -> String {
    command
        .caller
        .user_id
        .clone()
        .unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string())
}
