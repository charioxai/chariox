use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::unix_epoch_ms;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationDefinition {
    id: String,
    session_id: String,
    workflow_id: String,
    endpoint_id: String,
    alias: Option<String>,
    enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    route: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transport: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parser: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pairing_codes: Vec<WorkflowPublicationPairingCode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    trusted_senders: Vec<WorkflowPublicationTrustedSender>,
    created_by_user_id: String,
    created_at_ms: u64,
    updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationPairingCode {
    code_id: String,
    publication_id: String,
    pair_code_hash: String,
    created_by_user_id: String,
    created_at_ms: u64,
    expires_at_ms: Option<u64>,
    max_uses: Option<u32>,
    used_count: u32,
    revoked_at_ms: Option<u64>,
}

impl WorkflowPublicationPairingCode {
    pub fn new(
        code_id: impl Into<String>,
        publication_id: impl Into<String>,
        pair_code_hash: impl Into<String>,
        created_by_user_id: impl Into<String>,
        created_at_ms: u64,
        expires_at_ms: Option<u64>,
        max_uses: Option<u32>,
    ) -> Self {
        Self {
            code_id: code_id.into(),
            publication_id: publication_id.into(),
            pair_code_hash: pair_code_hash.into(),
            created_by_user_id: created_by_user_id.into(),
            created_at_ms,
            expires_at_ms,
            max_uses,
            used_count: 0,
            revoked_at_ms: None,
        }
    }

    pub fn code_id(&self) -> &str {
        &self.code_id
    }

    pub fn publication_id(&self) -> &str {
        &self.publication_id
    }

    pub fn pair_code_hash(&self) -> &str {
        &self.pair_code_hash
    }

    pub fn created_by_user_id(&self) -> &str {
        &self.created_by_user_id
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn expires_at_ms(&self) -> Option<u64> {
        self.expires_at_ms
    }

    pub fn max_uses(&self) -> Option<u32> {
        self.max_uses
    }

    pub fn used_count(&self) -> u32 {
        self.used_count
    }

    pub fn revoked_at_ms(&self) -> Option<u64> {
        self.revoked_at_ms
    }

    pub fn is_expired(&self, now_ms: u64) -> bool {
        self.expires_at_ms
            .is_some_and(|expires_at_ms| expires_at_ms <= now_ms)
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked_at_ms.is_some()
    }

    pub fn is_exhausted(&self) -> bool {
        self.max_uses
            .is_some_and(|max_uses| self.used_count >= max_uses)
    }

    pub fn mark_used(&mut self) {
        self.used_count = self.used_count.saturating_add(1);
    }

    pub fn revoke(&mut self, revoked_at_ms: u64) {
        self.revoked_at_ms = Some(revoked_at_ms);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationTrustedSender {
    sender_id: String,
    publication_id: String,
    display_name: Option<String>,
    credential_hash: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_transports: Vec<String>,
    created_at_ms: u64,
    last_used_at_ms: Option<u64>,
    expires_at_ms: Option<u64>,
    revoked_at_ms: Option<u64>,
}

impl WorkflowPublicationTrustedSender {
    pub fn new(
        sender_id: impl Into<String>,
        publication_id: impl Into<String>,
        display_name: Option<String>,
        credential_hash: impl Into<String>,
        allowed_transports: Vec<String>,
        created_at_ms: u64,
        expires_at_ms: Option<u64>,
    ) -> Self {
        Self {
            sender_id: sender_id.into(),
            publication_id: publication_id.into(),
            display_name,
            credential_hash: credential_hash.into(),
            allowed_transports,
            created_at_ms,
            last_used_at_ms: None,
            expires_at_ms,
            revoked_at_ms: None,
        }
    }

    pub fn sender_id(&self) -> &str {
        &self.sender_id
    }

    pub fn publication_id(&self) -> &str {
        &self.publication_id
    }

    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    pub fn credential_hash(&self) -> &str {
        &self.credential_hash
    }

    pub fn allowed_transports(&self) -> &[String] {
        &self.allowed_transports
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn last_used_at_ms(&self) -> Option<u64> {
        self.last_used_at_ms
    }

    pub fn expires_at_ms(&self) -> Option<u64> {
        self.expires_at_ms
    }

    pub fn revoked_at_ms(&self) -> Option<u64> {
        self.revoked_at_ms
    }

    pub fn is_expired(&self, now_ms: u64) -> bool {
        self.expires_at_ms
            .is_some_and(|expires_at_ms| expires_at_ms <= now_ms)
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked_at_ms.is_some()
    }

    pub fn allows_transport(&self, transport: &str) -> bool {
        self.allowed_transports.is_empty()
            || self
                .allowed_transports
                .iter()
                .any(|allowed| allowed == transport)
    }

    pub fn mark_used(&mut self, used_at_ms: u64) {
        self.last_used_at_ms = Some(used_at_ms);
    }

    pub fn revoke(&mut self, revoked_at_ms: u64) {
        self.revoked_at_ms = Some(revoked_at_ms);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationPairingCodeRecord {
    pub code: WorkflowPublicationPairingCode,
    pub pair_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationSenderCredential {
    pub sender: WorkflowPublicationTrustedSender,
    pub credential: String,
}

impl WorkflowPublicationDefinition {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        session_id: impl Into<String>,
        workflow_id: impl Into<String>,
        endpoint_id: impl Into<String>,
        alias: Option<String>,
        route: Option<String>,
        methods: Vec<String>,
        transport: Option<Value>,
        auth: Option<Value>,
        parser: Option<Value>,
        input_schema: Option<Value>,
        mode: Option<String>,
        created_by_user_id: impl Into<String>,
    ) -> Self {
        let now = unix_epoch_ms();
        Self {
            id: id.into(),
            session_id: session_id.into(),
            workflow_id: workflow_id.into(),
            endpoint_id: endpoint_id.into(),
            alias,
            enabled: true,
            route,
            methods,
            transport,
            auth,
            parser,
            input_schema,
            mode,
            pairing_codes: Vec::new(),
            trusted_senders: Vec::new(),
            created_by_user_id: created_by_user_id.into(),
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }

    pub fn endpoint_id(&self) -> &str {
        &self.endpoint_id
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn created_by_user_id(&self) -> &str {
        &self.created_by_user_id
    }

    pub fn pairing_codes(&self) -> &[WorkflowPublicationPairingCode] {
        &self.pairing_codes
    }

    pub fn pairing_code_mut(
        &mut self,
        code_id: &str,
    ) -> Option<&mut WorkflowPublicationPairingCode> {
        self.pairing_codes
            .iter_mut()
            .find(|code| code.code_id() == code_id)
    }

    pub fn trusted_senders(&self) -> &[WorkflowPublicationTrustedSender] {
        &self.trusted_senders
    }

    pub fn trusted_sender_mut(
        &mut self,
        sender_id: &str,
    ) -> Option<&mut WorkflowPublicationTrustedSender> {
        self.trusted_senders
            .iter_mut()
            .find(|sender| sender.sender_id() == sender_id)
    }

    pub fn trusted_sender_by_credential_hash_mut(
        &mut self,
        credential_hash: &str,
    ) -> Option<&mut WorkflowPublicationTrustedSender> {
        self.trusted_senders
            .iter_mut()
            .find(|sender| sender.credential_hash() == credential_hash)
    }

    pub fn add_pairing_code(
        &mut self,
        code: WorkflowPublicationPairingCode,
    ) -> WorkflowPublicationPairingCode {
        self.pairing_codes.push(code.clone());
        self.updated_at_ms = unix_epoch_ms();
        code
    }

    pub fn add_trusted_sender(
        &mut self,
        sender: WorkflowPublicationTrustedSender,
    ) -> WorkflowPublicationTrustedSender {
        self.trusted_senders.push(sender.clone());
        self.updated_at_ms = unix_epoch_ms();
        sender
    }

    pub fn disable(&mut self) {
        self.enabled = false;
        self.updated_at_ms = unix_epoch_ms();
    }
}
