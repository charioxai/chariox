//! Workflow publication, pairing-code, and sender credential mutations.
//!
//! This module owns public endpoint publication administration. Workflow graph design and run
//! administration stay in `workflow_admin`.

use super::*;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_create_publication(
        &self,
        request: crate::local::CreateWorkflowPublicationRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_endpoint_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            caller_user_id,
            "publish workflow endpoint",
        )?;
        let publication = self.session_store.write().create_workflow_publication(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.alias,
            request.route,
            request.methods,
            request.transport,
            request.auth,
            request.parser,
            request.input_schema,
            request.mode,
            caller_user_id.to_string(),
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowPublicationCreated {
            publication,
            session,
        })
    }

    pub(super) fn workflow_list_publications(
        &self,
        request: crate::local::ListWorkflowPublicationsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowPublicationsListed {
            publications: self
                .session_store
                .read()
                .list_workflow_publications(&request.session_id)?,
        })
    }

    pub(super) fn workflow_get_publication(
        &self,
        request: crate::local::GetWorkflowPublicationRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowPublication {
            publication: self
                .session_store
                .read()
                .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)?,
        })
    }

    pub(super) fn workflow_disable_publication(
        &self,
        request: crate::local::DisableWorkflowPublicationRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let publication = self
            .session_store
            .read()
            .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)?;
        if publication.created_by_user_id() != caller_user_id {
            return Err(Self::deny_owner(
                caller_user_id,
                publication.created_by_user_id(),
                format!("workflow publication `{}`", request.publication_ref),
                "disable workflow publication",
            ));
        }
        let publication = self
            .session_store
            .write()
            .disable_workflow_publication(&request.session_id, &request.publication_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowPublicationDisabled {
            publication,
            session,
        })
    }

    pub(super) fn workflow_create_publication_pair_code(
        &self,
        request: crate::local::CreateWorkflowPublicationPairCodeRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let publication = self
            .session_store
            .read()
            .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)?;
        if publication.created_by_user_id() != caller_user_id {
            return Err(Self::deny_owner(
                caller_user_id,
                publication.created_by_user_id(),
                format!("workflow publication `{}`", request.publication_ref),
                "create workflow publication pairing code",
            ));
        }
        let now_ms = crate::session::unix_epoch_ms();
        let expires_at_ms = request
            .expires_in_ms
            .map(|expires_in_ms| now_ms.saturating_add(expires_in_ms));
        let nonce = random_hex_id();
        let code_id = {
            let mut store = self.session_store.write();
            store.next_workflow_publication_pairing_code_id()
        };
        let pair_code = encode_workflow_publication_pair_code(&WorkflowPublicationPairCodeToken {
            version: 1,
            session_id: request.session_id.clone(),
            publication_id: publication.id().to_string(),
            code_id: code_id.clone(),
            nonce,
            issued_at_ms: now_ms,
            expires_at_ms,
            max_uses: request.max_uses,
        })?;
        let pair_code_hash = hash_secret(&pair_code);
        let code = self
            .session_store
            .write()
            .create_workflow_publication_pairing_code_with_id(
                &request.session_id,
                &request.publication_ref,
                code_id,
                &pair_code_hash,
                caller_user_id.to_string(),
                expires_at_ms,
                request.max_uses,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowPublicationPairCodeCreated {
            pair_code: crate::session::WorkflowPublicationPairingCodeRecord { code, pair_code },
            session,
        })
    }

    pub(super) fn workflow_redeem_publication_pair_code(
        &self,
        request: crate::local::RedeemWorkflowPublicationPairCodeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let token = decode_workflow_publication_pair_code(&request.pair_code)?;
        if token.session_id != request.session_id {
            return Err(DaemonError::LocalTransport {
                operation: "redeem workflow publication pairing code",
                message: "pairing code belongs to a different session".to_string(),
            });
        }
        let now_ms = crate::session::unix_epoch_ms();
        if token
            .expires_at_ms
            .is_some_and(|expires_at_ms| expires_at_ms <= now_ms)
        {
            return Err(DaemonError::LocalTransport {
                operation: "redeem workflow publication pairing code",
                message: "pairing code is expired".to_string(),
            });
        }
        let expires_at_ms = request
            .expires_in_ms
            .map(|expires_in_ms| now_ms.saturating_add(expires_in_ms));
        let credential = format!("arroba-publication-sender-v1.{}", random_hex_id());
        let pair_code_hash = hash_secret(&request.pair_code);
        let sender_credential = self
            .session_store
            .write()
            .redeem_workflow_publication_pairing_code(
                &request.session_id,
                &request.publication_ref,
                &token.code_id,
                &pair_code_hash,
                &credential,
                request.display_name,
                request.allowed_transports,
                expires_at_ms,
                now_ms,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowPublicationSenderPaired {
            sender_credential,
            session,
        })
    }

    pub(super) fn workflow_list_publication_senders(
        &self,
        request: crate::local::ListWorkflowPublicationSendersRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_publication_owner(
            &request.session_id,
            &request.publication_ref,
            caller_user_id,
            "list workflow publication senders",
        )?;
        Ok(LocalDaemonResponse::WorkflowPublicationSendersListed {
            senders: self
                .session_store
                .read()
                .list_workflow_publication_senders(&request.session_id, &request.publication_ref)?,
        })
    }

    pub(super) fn workflow_revoke_publication_sender(
        &self,
        request: crate::local::RevokeWorkflowPublicationSenderRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_publication_owner(
            &request.session_id,
            &request.publication_ref,
            caller_user_id,
            "revoke workflow publication sender",
        )?;
        let sender = self
            .session_store
            .write()
            .revoke_workflow_publication_sender(
                &request.session_id,
                &request.publication_ref,
                &request.sender_ref,
                crate::session::unix_epoch_ms(),
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowPublicationSenderRevoked { sender, session })
    }

    pub(super) fn workflow_authenticate_publication_sender(
        &self,
        request: crate::local::AuthenticateWorkflowPublicationSenderRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(
            LocalDaemonResponse::WorkflowPublicationSenderAuthenticated {
                sender: self
                    .session_store
                    .write()
                    .authenticate_workflow_publication_sender(
                        &request.session_id,
                        &request.publication_ref,
                        &request.credential,
                        &request.transport,
                        crate::session::unix_epoch_ms(),
                    )?,
            },
        )
    }

    fn ensure_workflow_publication_owner(
        &self,
        session_id: &str,
        publication_ref: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let publication = self
            .session_store
            .read()
            .resolve_workflow_publication_ref(session_id, publication_ref)?;
        if publication.created_by_user_id() == user_id {
            Ok(())
        } else {
            Err(Self::deny_owner(
                user_id,
                publication.created_by_user_id(),
                format!("workflow publication `{publication_ref}`"),
                operation,
            ))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkflowPublicationPairCodeToken {
    version: u8,
    session_id: String,
    publication_id: String,
    code_id: String,
    nonce: String,
    issued_at_ms: u64,
    #[serde(default)]
    expires_at_ms: Option<u64>,
    #[serde(default)]
    max_uses: Option<u32>,
}

fn encode_workflow_publication_pair_code(
    token: &WorkflowPublicationPairCodeToken,
) -> Result<String, DaemonError> {
    let payload = serde_json::to_vec(token).map_err(|error| DaemonError::LocalTransport {
        operation: "encode workflow publication pairing code",
        message: error.to_string(),
    })?;
    Ok(format!(
        "arroba-publication-pair-v1.{}",
        URL_SAFE_NO_PAD.encode(payload)
    ))
}

fn decode_workflow_publication_pair_code(
    token: &str,
) -> Result<WorkflowPublicationPairCodeToken, DaemonError> {
    let payload = token
        .trim()
        .strip_prefix("arroba-publication-pair-v1.")
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "decode workflow publication pairing code",
            message: "pairing code has an unsupported format".to_string(),
        })?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "decode workflow publication pairing code",
            message: error.to_string(),
        })?;
    let decoded =
        serde_json::from_slice::<WorkflowPublicationPairCodeToken>(&bytes).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "decode workflow publication pairing code",
                message: error.to_string(),
            }
        })?;
    if decoded.version != 1 {
        return Err(DaemonError::LocalTransport {
            operation: "decode workflow publication pairing code",
            message: format!(
                "unsupported workflow publication pairing code version {}",
                decoded.version
            ),
        });
    }
    Ok(decoded)
}

fn random_hex_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hash_secret(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}
