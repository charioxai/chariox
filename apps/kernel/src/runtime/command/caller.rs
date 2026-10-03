use serde::{Deserialize, Serialize};

use chariox_relay::auth::RelaySubjectKind;
use chariox_relay::protocol::RelayCallerIdentity;

use crate::local::KernelConnectionClass;
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
    /// Protocol 402: the class of the connection this caller was admitted on.
    /// Absent for the kernel's own commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_class: Option<KernelConnectionClass>,
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
            connection_class: None,
        }
    }

    pub fn with_connection_class(mut self, connection_class: KernelConnectionClass) -> Self {
        self.connection_class = Some(connection_class);
        self
    }

    /// A relay request's caller: its authenticated relay identity, or an
    /// unverified relay client when the relay supplied none.
    pub fn for_relay_request(identity: Option<RelayCallerIdentity>) -> Self {
        identity.map(Self::from_relay_identity).unwrap_or_else(|| {
            Self::for_source(&KernelCommandSource::RelayClient)
                .with_connection_class(relay_connection_class(None))
        })
    }

    pub fn from_relay_identity(identity: RelayCallerIdentity) -> Self {
        let connection_class = relay_connection_class(Some(&identity));
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
            connection_class: Some(connection_class),
        }
    }
}

/// The class of a relay connection, for its requests and subscriptions: a
/// client with a user id is a terminal (web, remote TUI); kernels, machines
/// and hosted services are relay peers; a client without a user id, or no
/// identity, is unauthenticated.
pub(crate) fn relay_connection_class(
    identity: Option<&RelayCallerIdentity>,
) -> KernelConnectionClass {
    match identity.map(|identity| (identity.subject_kind, identity.user_id.is_some())) {
        Some((RelaySubjectKind::Client, true)) => KernelConnectionClass::Terminal,
        Some((RelaySubjectKind::Client, false)) | None => KernelConnectionClass::Unauthenticated,
        Some((
            RelaySubjectKind::Kernel | RelaySubjectKind::Machine | RelaySubjectKind::Service,
            _,
        )) => KernelConnectionClass::RelayPeer,
    }
}

pub(crate) fn command_caller_user_id(command: &KernelCommand) -> String {
    command
        .caller
        .user_id
        .clone()
        .unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string())
}

impl KernelCommand {
    /// This identity was supplied by the local transport or authenticated relay
    /// admission. It is never read from an interaction's response payload.
    pub(crate) fn is_terminal_caller(&self) -> bool {
        self.caller.metaagent_id.is_none()
            && self.caller.connection_class == Some(KernelConnectionClass::Terminal)
    }
}
