use super::*;

pub fn apply_workflow_code_provider_rebindings(
    definition: &mut WorkflowCodeDefinition,
    rebindings: &[WorkflowCodeProviderRebinding],
) -> Result<(), crate::DaemonError> {
    if rebindings.is_empty() {
        return Ok(());
    }
    let mut seen = BTreeSet::new();
    for rebinding in rebindings {
        let node_handle = rebinding.node.trim();
        if node_handle.is_empty() {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.rebind",
                message: "provider rebinding node handle must not be empty".to_string(),
            });
        }
        if !seen.insert(node_handle.to_string()) {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.rebind",
                message: format!("duplicate provider rebinding for node `{node_handle}`"),
            });
        }
        if rebinding.provider.trim().is_empty() {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.rebind",
                message: format!(
                    "provider rebinding for node `{node_handle}` must include provider"
                ),
            });
        }
        let node = definition
            .nodes
            .iter_mut()
            .find(|node| node.handle == node_handle)
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.rebind",
                message: format!("provider rebinding references unknown node `{node_handle}`"),
            })?;
        match &mut node.agent {
            WorkflowCodeAgentBinding::Create(agent) => {
                agent.provider = rebinding.provider.trim().to_string();
                if let Some(model) = rebinding.model.as_deref() {
                    let model = model.trim();
                    agent.model = if model.is_empty() {
                        None
                    } else {
                        Some(model.to_string())
                    };
                }
                if let Some(effort) = rebinding.effort.as_deref() {
                    let effort = effort.trim();
                    agent.effort = if effort.is_empty() {
                        None
                    } else {
                        Some(effort.to_string())
                    };
                }
                if let Some(account_profile) = rebinding.account_profile.as_deref() {
                    let account_profile = account_profile.trim();
                    agent.account_profile =
                        if account_profile.is_empty() || account_profile == "default" {
                            None
                        } else {
                            Some(account_profile.to_string())
                        };
                }
            }
            WorkflowCodeAgentBinding::Existing(_) => {
                return Err(crate::DaemonError::LocalTransport {
                    operation: "workflow_code.rebind",
                    message: format!(
                        "provider rebinding for node `{node_handle}` targets an existing-agent binding"
                    ),
                });
            }
        }
    }
    Ok(())
}

pub fn apply_workflow_code_agent_rebindings(
    definition: &mut WorkflowCodeDefinition,
    rebindings: &[WorkflowCodeAgentRebinding],
) -> Result<(), crate::DaemonError> {
    if rebindings.is_empty() {
        return Ok(());
    }
    let mut seen = BTreeSet::new();
    for rebinding in rebindings {
        let node_handle = rebinding.node.trim();
        if node_handle.is_empty() {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.rebind",
                message: "agent rebinding node handle must not be empty".to_string(),
            });
        }
        if !seen.insert(node_handle.to_string()) {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.rebind",
                message: format!("duplicate agent rebinding for node `{node_handle}`"),
            });
        }
        let agent_ref = rebinding.agent_ref.trim();
        if agent_ref.is_empty() {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.rebind",
                message: format!("agent rebinding for node `{node_handle}` must include agent_ref"),
            });
        }
        let node = definition
            .nodes
            .iter_mut()
            .find(|node| node.handle == node_handle)
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.rebind",
                message: format!("agent rebinding references unknown node `{node_handle}`"),
            })?;
        match &node.agent {
            WorkflowCodeAgentBinding::Create(_) => {
                node.agent = WorkflowCodeAgentBinding::Existing(WorkflowCodeExistingAgent {
                    agent_ref: agent_ref.to_string(),
                });
            }
            WorkflowCodeAgentBinding::Existing(_) => {
                return Err(crate::DaemonError::LocalTransport {
                    operation: "workflow_code.rebind",
                    message: format!(
                        "agent rebinding for node `{node_handle}` targets an existing-agent binding"
                    ),
                });
            }
        }
    }
    Ok(())
}
