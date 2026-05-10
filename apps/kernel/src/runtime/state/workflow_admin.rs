//! Workflow administrative mutations.
//!
//! Owns workflow CRUD, endpoint edits, watchdog updates, and queue-facing commands that alter
//! workflow definitions rather than executing an individual node.

use super::*;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

impl KernelRuntimeOwnedState {
    fn deny_owner(
        user_id: &str,
        owner_user_id: &str,
        resource: String,
        operation: &'static str,
    ) -> DaemonError {
        DaemonError::OwnershipAccessDenied {
            user_id: user_id.to_string(),
            owner_user_id: owner_user_id.to_string(),
            resource,
            operation,
        }
    }

    fn ensure_workflow_node_owner(
        &self,
        session_id: &str,
        workflow_ref: &str,
        node_id: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let node = workflow
            .node(node_id)
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                node_id: node_id.to_string(),
            })?;
        if node.owner_user_id() == user_id {
            Ok(())
        } else {
            Err(Self::deny_owner(
                user_id,
                node.owner_user_id(),
                format!("workflow node `{node_id}`"),
                operation,
            ))
        }
    }

    pub(super) fn ensure_workflow_endpoint_owner(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            workflow_ref,
            endpoint_ref,
        )?;
        if endpoint.owner_user_id() == user_id {
            Ok(())
        } else {
            Err(Self::deny_owner(
                user_id,
                endpoint.owner_user_id(),
                format!("workflow endpoint `{endpoint_ref}`"),
                operation,
            ))
        }
    }

    fn ensure_workflow_revision(
        &self,
        session_id: &str,
        workflow_ref: &str,
        expected_revision: Option<u64>,
    ) -> Result<(), DaemonError> {
        let Some(expected_revision) = expected_revision else {
            return Ok(());
        };
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let current_revision = workflow.revision();
        if current_revision == expected_revision {
            Ok(())
        } else {
            Err(DaemonError::WorkflowRevisionConflict {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                expected_revision,
                current_revision,
            })
        }
    }

    fn ensure_workflow_edge_incident_to_owner(
        &self,
        session_id: &str,
        workflow_ref: &str,
        edge_id: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let edge = workflow
            .edge(edge_id)
            .ok_or_else(|| DaemonError::WorkflowEdgeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                edge_id: edge_id.to_string(),
            })?;
        let from_owner = workflow
            .node(edge.from_node_id())
            .map(|node| node.owner_user_id());
        let to_owner = workflow
            .node(edge.to_node_id())
            .map(|node| node.owner_user_id());
        if from_owner == Some(user_id) || to_owner == Some(user_id) {
            Ok(())
        } else {
            Err(Self::deny_owner(
                user_id,
                edge.created_by_user_id(),
                format!("workflow edge `{edge_id}`"),
                operation,
            ))
        }
    }

    pub(super) fn workflow_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.session_snapshot(session_id)
    }

    pub(super) fn workflow_create_workflow(
        &self,
        request: crate::local::CreateWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .session_store
            .write()
            .create_workflow(&request.session_id, request.alias)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowCreated { workflow, session })
    }

    pub(super) fn workflow_apply_design_op(
        &self,
        request: crate::local::ApplyWorkflowDesignOpRequest,
        caller_user_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.session_store.write().apply_workflow_design_op(
            &request.session_id,
            request.op,
            caller_user_id.to_string(),
        )?;
        self.workflow_session(&request.session_id)
    }

    pub(super) fn workflow_alias_workflow(
        &self,
        request: crate::local::AliasWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self.session_store.write().assign_workflow_alias(
            &request.session_id,
            &request.workflow_ref,
            request.alias,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowAliased { workflow, session })
    }

    pub(super) fn workflow_list_workflows(
        &self,
        request: crate::local::ListWorkflowsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowsListed {
            workflows: self
                .session_store
                .read()
                .list_workflows(&request.session_id)?,
        })
    }

    pub(super) fn workflow_resolve_workflow(
        &self,
        request: crate::local::ResolveWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowResolved {
            workflow: self
                .session_store
                .read()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?,
        })
    }

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

    pub(super) fn workflow_create_endpoint(
        &self,
        request: crate::local::CreateWorkflowEndpointRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.entry_node_id,
            caller_user_id,
            "create workflow endpoint",
        )?;
        let endpoint = self.session_store.write().create_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.entry_node_id,
            request.alias,
        )?;
        let endpoint = self.session_store.write().set_workflow_endpoint_owner(
            &request.session_id,
            &request.workflow_ref,
            endpoint.id(),
            caller_user_id.to_string(),
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_alias_endpoint(
        &self,
        request: crate::local::AliasWorkflowEndpointRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_endpoint_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            caller_user_id,
            "alias workflow endpoint",
        )?;
        let endpoint = self.session_store.write().assign_workflow_endpoint_alias(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.alias,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointAliased {
            endpoint,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_bind_endpoint(
        &self,
        request: crate::local::BindWorkflowEndpointRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_endpoint_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            caller_user_id,
            "bind workflow endpoint",
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.entry_node_id,
            caller_user_id,
            "bind workflow endpoint",
        )?;
        let endpoint = self.session_store.write().bind_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            &request.entry_node_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointBound {
            endpoint,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_add_node(
        &self,
        request: crate::local::AddWorkflowNodeRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let agent = if let Some(agent) = self
            .agent_store
            .get_session_agents(&request.session_id)
            .into_iter()
            .find(|agent| agent.id() == request.agent_id)
        {
            agent
        } else {
            return Err(DaemonError::AgentNotFound {
                agent_id: request.agent_id,
            });
        };
        if agent.owner_user_id() != caller_user_id {
            return Err(Self::deny_owner(
                caller_user_id,
                agent.owner_user_id(),
                format!("agent `{}`", request.agent_id),
                "add workflow node",
            ));
        }
        let node = self.session_store.write().add_workflow_node_owned(
            &request.session_id,
            &request.workflow_ref,
            &request.agent_id,
            caller_user_id.to_string(),
            request.agent_id.clone(),
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeAdded {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_remove_node(
        &self,
        request: crate::local::RemoveWorkflowNodeRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "remove workflow node",
        )?;
        let node = self.session_store.write().remove_workflow_node(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeRemoved {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_update_node_instructions(
        &self,
        request: crate::local::UpdateWorkflowNodeInstructionsRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "update workflow node instructions",
        )?;
        let node = self
            .session_store
            .write()
            .update_workflow_node_instructions(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.instructions,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_set_node_can_complete_run(
        &self,
        request: crate::local::SetWorkflowNodeCanCompleteRunRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "set workflow node completion policy",
        )?;
        let node = self
            .session_store
            .write()
            .set_workflow_node_can_complete_run(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_complete_workflow_run,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_set_node_can_emit_intermediate_output(
        &self,
        request: crate::local::SetWorkflowNodeCanEmitIntermediateOutputRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "set workflow node intermediate output policy",
        )?;
        let node = self
            .session_store
            .write()
            .set_workflow_node_can_emit_intermediate_output(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_emit_intermediate_workflow_run_output,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    pub(super) fn workflow_set_node_intermediate_output_schema(
        &self,
        request: crate::local::SetWorkflowNodeIntermediateOutputSchemaRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "set workflow node intermediate output schema",
        )?;
        let node = self
            .session_store
            .write()
            .set_workflow_node_intermediate_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.intermediate_output_schema_ref,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    pub(super) fn workflow_set_node_max_turns(
        &self,
        request: crate::local::SetWorkflowNodeMaxTurnsRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "set workflow node max turns",
        )?;
        let node = self.session_store.write().set_workflow_node_max_turns(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            request.max_turns,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_add_edge(
        &self,
        request: crate::local::AddWorkflowEdgeRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let from_owner = workflow
            .node(&request.from_node_id)
            .map(|node| node.owner_user_id())
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: request.session_id.clone(),
                workflow_id: workflow.id().to_string(),
                node_id: request.from_node_id.clone(),
            })?;
        let to_owner = workflow
            .node(&request.to_node_id)
            .map(|node| node.owner_user_id())
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: request.session_id.clone(),
                workflow_id: workflow.id().to_string(),
                node_id: request.to_node_id.clone(),
            })?;
        if from_owner != caller_user_id && to_owner != caller_user_id {
            return Err(Self::deny_owner(
                caller_user_id,
                from_owner,
                format!(
                    "workflow edge `{} -> {}`",
                    request.from_node_id, request.to_node_id
                ),
                "add workflow edge",
            ));
        }
        let edge = self.session_store.write().add_workflow_edge_owned(
            &request.session_id,
            &request.workflow_ref,
            &request.from_node_id,
            &request.to_node_id,
            caller_user_id.to_string(),
            request.output_schema_ref,
            request.validation_policy,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeAdded {
            edge,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_remove_edge(
        &self,
        request: crate::local::RemoveWorkflowEdgeRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_edge_incident_to_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.edge_id,
            caller_user_id,
            "remove workflow edge",
        )?;
        let edge = self.session_store.write().remove_workflow_edge(
            &request.session_id,
            &request.workflow_ref,
            &request.edge_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeRemoved {
            edge,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_update_canvas_layout(
        &self,
        request: crate::local::UpdateWorkflowCanvasLayoutRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let layout = self.session_store.write().update_workflow_canvas_layout(
            &request.session_id,
            &request.workflow_ref,
            request.patches,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowCanvasLayoutUpdated {
            layout,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_set_flush_context(
        &self,
        request: crate::local::SetWorkflowFlushContextRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self
            .session_store
            .write()
            .set_workflow_flush_agent_context_before_run(
                &request.session_id,
                &request.workflow_ref,
                request.flush_agent_context_before_run,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session })
    }

    pub(super) fn workflow_set_run_output_schema(
        &self,
        request: crate::local::SetWorkflowRunOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self
            .session_store
            .write()
            .set_workflow_run_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                request.run_output_schema_ref,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session })
    }

    pub(super) fn workflow_set_intermediate_output_schema(
        &self,
        request: crate::local::SetWorkflowIntermediateOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self
            .session_store
            .write()
            .set_workflow_intermediate_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                request.intermediate_output_schema_ref,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { workflow, session })
    }

    pub(super) fn workflow_set_launch_policy(
        &self,
        request: crate::local::SetWorkflowLaunchPolicyRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session = self
            .session_store
            .write()
            .set_workflow_launch_policy(&request.session_id, request.policy)?;
        let mut session = session;
        session.set_agents(self.agent_store.get_session_agents(&request.session_id));
        self.project_session_runtime_view(&mut session);
        self.session_projection.update(session.clone());
        Ok(LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session })
    }

    pub(super) fn workflow_list_runs(
        &self,
        request: crate::local::ListWorkflowRunsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowRunsListed {
            workflow_runs: self
                .session_store
                .read()
                .list_workflow_runs(&request.session_id, request.workflow_ref.as_deref())?,
        })
    }

    pub(super) fn workflow_get_run(
        &self,
        request: crate::local::GetWorkflowRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowRun {
            workflow_run: self
                .session_store
                .read()
                .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?,
        })
    }

    pub(super) fn workflow_create_watchdog(
        &self,
        request: crate::local::CreateWorkflowWatchdogRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self.session_store.write().create_workflow_watchdog(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.interval_seconds,
            request.invocation_prompt,
            request.policy,
            if request.max_wakeups_configured {
                Some(request.max_wakeups)
            } else {
                None
            },
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogCreated {
            watchdog,
            workflow,
            endpoint,
            session,
        })
    }

    pub(super) fn workflow_list_watchdogs(
        &self,
        request: crate::local::ListWorkflowWatchdogsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowWatchdogsListed {
            watchdogs: self
                .session_store
                .read()
                .list_workflow_watchdogs(&request.session_id, request.workflow_ref.as_deref())?,
        })
    }

    pub(super) fn workflow_set_watchdog_enabled(
        &self,
        request: crate::local::SetWorkflowWatchdogEnabledRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self.session_store.write().set_workflow_watchdog_enabled(
            &request.session_id,
            &request.watchdog_ref,
            request.enabled,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogUpdated { watchdog, session })
    }

    pub(super) fn workflow_remove_watchdog(
        &self,
        request: crate::local::RemoveWorkflowWatchdogRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self
            .session_store
            .write()
            .remove_workflow_watchdog(&request.session_id, &request.watchdog_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogRemoved { watchdog, session })
    }

    pub(super) fn workflow_list_queued_launches(
        &self,
        request: crate::local::ListQueuedWorkflowLaunchesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchesListed {
            queued_launches: self
                .session_store
                .read()
                .list_queued_workflow_launches(&request.session_id)?,
        })
    }

    pub(super) fn workflow_remove_queued_launch(
        &self,
        request: crate::local::RemoveQueuedWorkflowLaunchRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let queued_launch = self
            .session_store
            .write()
            .remove_queued_workflow_launch(&request.session_id, &request.queue_item_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchRemoved {
            queued_launch,
            session,
        })
    }

    pub(super) fn workflow_clear_queued_launches(
        &self,
        request: crate::local::ClearQueuedWorkflowLaunchesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let queued_launches = self
            .session_store
            .write()
            .clear_queued_workflow_launches(&request.session_id)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchesCleared {
            queued_launches,
            session,
        })
    }

    pub(super) fn workflow_validate_output(
        &self,
        request: crate::local::ValidateWorkflowOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let warning = crate::transport::runtime_tools::validate_workflow_output_schema(
            &request.output_schema_ref,
            &request.output_json,
        )
        .err();
        Ok(LocalDaemonResponse::WorkflowOutputValidated {
            valid: warning.is_none(),
            warning,
        })
    }

    pub(super) fn workflow_ack_turn(
        &self,
        request: crate::local::AckWorkflowTurnRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow_run_id = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
            .id()
            .to_string();
        let workflow_run = self.session_store.write().ack_workflow_turn(
            &request.session_id,
            &workflow_run_id,
            &request.workflow_node_run_id,
            &request.delivery_token,
        )?;
        let event = crate::session::WorkflowRuntimeToolCallEvent::new(
            crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL.to_string(),
            serde_json::json!({"delivery_token": request.delivery_token}).to_string(),
            Some(
                serde_json::json!({
                    "workflow_run_id": workflow_run.id(),
                    "workflow_node_run_id": request.workflow_node_run_id,
                    "state": "acknowledged",
                    "next_action": "Continue this same workflow turn. This acknowledgement is not the final answer. If this turn requires final workflow run output, call validate_and_submit_workflow_run_output before stopping; otherwise emit the required final fenced json block before stopping.",
                })
                .to_string(),
            ),
            true,
        );
        let _ = self
            .session_store
            .write()
            .record_workflow_runtime_tool_call(
                &request.session_id,
                &request.workflow_node_run_id,
                event,
            );
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&request.session_id, &workflow_run_id)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowTurnAcknowledged {
            workflow_run,
            session,
        })
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
