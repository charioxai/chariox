use super::*;

impl KernelRuntimeState {
    pub(super) fn set_resource_revoke(
        &self,
        run_id: &str,
        args: Value,
    ) -> Result<Value, DaemonError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            resource_id: String,
            revoke: ResourceRevokeMethod,
        }
        let args: Args = serde_json::from_value(args)
            .map_err(|_| error("invalid set_resource_revoke arguments"))?;
        let authority = self.resource_register_authority(run_id, "", "set_revoke")?;
        let store = &self.owned.durable_state_store;
        let owner = &authority.owner_user_id;
        let mut row = store
            .created_resource(owner, &authority.project_id, &args.resource_id)?
            .ok_or_else(|| error("resource not found in this project"))?;
        self.resource_register_authority(run_id, &row.service, &row.kind)?;
        if !matches!(
            row.state,
            CreatedResourceState::Active | CreatedResourceState::RevokeFailed
        ) {
            return Err(error(
                "cannot change the revoke method after revocation has started or completed",
            ));
        }
        RecordArgs {
            request_id: row.id.clone(),
            service: row.service.clone(),
            kind: row.kind.clone(),
            name: row.name.clone(),
            url: row.url.clone(),
            credential_id: row.credential_id.clone(),
            revoke: args.revoke.clone(),
        }
        .validate(self)?;
        let expected = row.clone();
        row.revoke = args.revoke;
        store.change_created_resource(owner, &row, Some(&expected))?;
        Ok(json!({"resource":row}))
    }

    pub(super) async fn revoke_created_resource(
        &self,
        run_id: &str,
        args: Value,
    ) -> Result<Value, DaemonError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            resource_id: String,
        }
        let args: Args =
            serde_json::from_value(args).map_err(|_| error("invalid revoke_resource arguments"))?;
        bounded(&args.resource_id)?;
        let authority = self.resource_register_authority(run_id, "", "revoke")?;
        let store = &self.owned.durable_state_store;
        let owner = &authority.owner_user_id;
        let mut row = store
            .created_resource(owner, &authority.project_id, &args.resource_id)?
            .ok_or_else(|| error("resource not found in this project"))?;
        let target = match &row.revoke {
            ResourceRevokeMethod::Http { request } => request.url.as_str(),
            ResourceRevokeMethod::Url { url } => url.as_str(),
            ResourceRevokeMethod::Unavailable { .. } => row.service.as_str(),
        };
        self.resource_register_authority(run_id, target, "revoke")?;
        match row.state {
            CreatedResourceState::Revoked => return Ok(json!({"resource_id":row.id,"status":"revoked"})),
            CreatedResourceState::Revoking => return Err(error("revocation is already in progress; wait for its result before taking further action")),
            CreatedResourceState::Active | CreatedResourceState::RevokeFailed => {}
        }
        let mut request = match row.revoke.clone() {
            ResourceRevokeMethod::Url { url } => {
                return Ok(json!({"resource_id":row.id,"status":"owner_action_required","url":url}))
            }
            ResourceRevokeMethod::Unavailable { reason } => {
                return Ok(
                    json!({"resource_id":row.id,"status":"revoke_unavailable","reason":reason}),
                )
            }
            ResourceRevokeMethod::Http { request } => request,
        };
        if self.provider_account_authority_owner_user_id(owner)
            != crate::session::DEFAULT_LOCAL_USER_ID
        {
            return Err(error("the owner's Vault is not available on this kernel"));
        }
        let service = self.home_runtime_secret_service()?;
        service.validate_http_request_target(&request.credential_id, &request.url)?;
        let _unlock = self
            .ensure_vault_unlocked_for_agent(
                &authority.session_id,
                &authority.agent_id,
                "resource_register.revoke",
            )
            .await?;
        let current = self.resource_register_authority(run_id, &request.url, "revoke")?;
        if !current.same_turn(&authority) {
            return Err(error(
                "resource authority changed while waiting for the Vault; retry in the current turn",
            ));
        }
        // Preparing performs injection and validation locally. Neither response content nor errors
        // from the service are sent to the model: a revoke endpoint may echo its credentials.
        request.max_response_bytes = request.max_response_bytes.min(65536);
        let target = request.url.clone();
        let prepared = service.prepare_http_request_with_credential(request)?;
        let expected = row.clone();
        row.state = CreatedResourceState::Revoking;
        store.change_created_resource(owner, &row, Some(&expected))?;
        let claimed = row.clone();
        let state = self.clone();
        let run_id = run_id.to_owned();

        let admitted = authority.clone();
        let response = tokio::task::spawn_blocking(move || {
            let current = state.resource_register_authority(&run_id, &target, "revoke")?;
            if !current.same_turn(&admitted) {
                return Err(error("resource authority changed before HTTP dispatch"));
            }
            prepared.execute()
        })
        .await;
        row.last_http_status = match &response {
            Ok(Ok(response)) => Some(response.status),
            _ => None,
        };
        let success = row
            .last_http_status
            .is_some_and(|status| (200..300).contains(&status));
        row.state = if success {
            CreatedResourceState::Revoked
        } else {
            CreatedResourceState::RevokeFailed
        };
        row.revoked_at_ms = success.then(crate::session::unix_epoch_ms);
        store.change_created_resource(owner, &row, Some(&claimed))?;
        if success {
            Ok(json!({"resource_id":row.id,"status":"revoked","http_status":row.last_http_status}))
        } else {
            Ok(
                json!({"resource_id":row.id,"status":"revoke_failed","http_status":row.last_http_status,
                "reason":"The service did not confirm revocation. Check its settings before retrying; a failed connection may have applied the request."}),
            )
        }
    }
}
