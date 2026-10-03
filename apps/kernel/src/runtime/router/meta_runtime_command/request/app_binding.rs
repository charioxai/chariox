use super::*;

pub(in crate::runtime::router::meta_runtime_command) fn meta_app_binding_request(
    session: &crate::session::RuntimeSession,
    metaagent: &crate::agent::AgentInstance,
    args: &[String],
    agents: &[crate::agent::AgentInstance],
) -> Result<LocalDaemonRequest, DaemonError> {
    if args.len() != 4 || args[1] != "app" || !matches!(args[0].as_str(), "grant" | "revoke") {
        return Err(meta_command_error(
            "usage: extension grant|revoke app <owned-agent-ref> <installation-id>",
        ));
    }
    let target = if args[2] == metaagent.id()
        || args[2] == metaagent.agent_ref()
        || Some(args[2].as_str()) == metaagent.alias()
    {
        metaagent.clone()
    } else {
        meta_owned_regular_agent_from_session(agents, metaagent, &args[2])?
    };
    crate::extension::ExtensionGrant::app(&args[3]).validate_app_binding()?;
    if args[0] == "grant" {
        Ok(LocalDaemonRequest::GrantAgentExtension(
            GrantAgentExtensionRequest {
                workspace_id: Some(session.workspace_id().into()),
                agent_ref: target.id().into(),
                kind: ExtensionKind::App,
                name: args[3].clone(),
                environment: None,
                credential: None,
                max_safety: None,
            },
        ))
    } else {
        Ok(LocalDaemonRequest::RevokeAgentExtension(
            RevokeAgentExtensionRequest {
                agent_ref: target.id().into(),
                kind: ExtensionKind::App,
                name: args[3].clone(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_binding_parser_preserves_meta_self_access_and_rejects_foreign_targets() {
        let session = crate::session::RuntimeSession::new(
            "session",
            None,
            "workspace",
            "/repo",
            "machine",
            "kernel",
        );
        let mut agent = crate::agent::AgentInstance::new(
            "meta",
            "meta-ref",
            "session",
            None,
            "dev-stub",
            None,
            None,
            None,
            crate::agent::GridPosition::new(0, 0, 1, 1),
        );
        agent.activate_meta_mode(None);
        let args = ["grant", "app", "meta-ref", "installed"].map(String::from);
        let request = meta_app_binding_request(&session, &agent, &args, &[]).unwrap();
        assert!(
            matches!(request, LocalDaemonRequest::GrantAgentExtension(GrantAgentExtensionRequest { kind: ExtensionKind::App, agent_ref, .. }) if agent_ref == "meta")
        );
        let mut invalid = args.to_vec();
        invalid[2] = "other-agent".into();
        assert!(meta_app_binding_request(&session, &agent, &invalid, &[]).is_err());
        let mut extra = args.to_vec();
        extra.extend(["--credential".into(), "secret".into()]);
        assert!(meta_app_binding_request(&session, &agent, &extra, &[]).is_err());
    }
}
