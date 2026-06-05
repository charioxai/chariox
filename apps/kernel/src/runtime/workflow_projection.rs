use crate::session::{RuntimeSession, WorkflowDefinition, WorkflowRun};
use crate::DaemonError;

pub(crate) fn projected_workflow_id(
    session: &RuntimeSession,
    workflow_ref: Option<&str>,
) -> Result<Option<String>, DaemonError> {
    workflow_ref
        .map(|reference| projected_resolve_workflow(session, reference))
        .transpose()
        .map(|workflow| workflow.map(|workflow| workflow.id().to_string()))
}

pub(crate) fn projected_resolve_workflow(
    session: &RuntimeSession,
    workflow_ref: &str,
) -> Result<WorkflowDefinition, DaemonError> {
    let normalized_ref = workflow_ref.trim().to_lowercase();
    if let Some(workflow) = session
        .workflows()
        .iter()
        .find(|workflow| workflow.id() == normalized_ref)
    {
        return Ok(workflow.clone());
    }
    if let Some(workflow) = session
        .workflows()
        .iter()
        .find(|workflow| workflow.alias() == Some(normalized_ref.as_str()))
    {
        return Ok(workflow.clone());
    }
    let id_matches = session
        .workflows()
        .iter()
        .filter(|workflow| workflow.id().starts_with(&normalized_ref))
        .cloned()
        .collect::<Vec<_>>();
    if id_matches.len() == 1 {
        return Ok(id_matches[0].clone());
    }
    let alias_matches = session
        .workflows()
        .iter()
        .filter(|workflow| {
            workflow
                .alias()
                .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
        })
        .cloned()
        .collect::<Vec<_>>();
    if alias_matches.len() == 1 {
        return Ok(alias_matches[0].clone());
    }
    Err(DaemonError::WorkflowNotFound {
        session_id: session.id().to_string(),
        workflow_id: workflow_ref.to_string(),
    })
}

pub(crate) fn projected_resolve_workflow_run(
    session: &RuntimeSession,
    workflow_run_ref: &str,
) -> Result<WorkflowRun, DaemonError> {
    let normalized_ref = workflow_run_ref.trim().to_lowercase();
    if let Some(workflow_run) = session
        .workflow_runs()
        .iter()
        .find(|workflow_run| workflow_run.id() == normalized_ref)
    {
        return Ok(workflow_run.clone());
    }
    let id_matches = session
        .workflow_runs()
        .iter()
        .filter(|workflow_run| workflow_run.id().starts_with(&normalized_ref))
        .cloned()
        .collect::<Vec<_>>();
    if id_matches.len() == 1 {
        return Ok(id_matches[0].clone());
    }
    Err(DaemonError::WorkflowRunNotFound {
        session_id: session.id().to_string(),
        workflow_run_id: workflow_run_ref.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with_workflows() -> RuntimeSession {
        let mut session =
            RuntimeSession::new("session-1", None, "/repo", "/repo", "machine-1", "daemon-1");
        session.create_workflow(WorkflowDefinition::new(
            "workflow-alpha",
            Some("deploy".to_string()),
        ));
        session.create_workflow(WorkflowDefinition::new(
            "workflow-beta",
            Some("debug".to_string()),
        ));
        session.create_workflow_run(WorkflowRun::new(
            "run-alpha",
            "workflow-alpha",
            "endpoint-1",
            "node-1",
            None,
            None,
            Vec::new(),
            Vec::new(),
        ));
        session.create_workflow_run(WorkflowRun::new(
            "run-beta",
            "workflow-beta",
            "endpoint-1",
            "node-1",
            None,
            None,
            Vec::new(),
            Vec::new(),
        ));
        session
    }

    #[test]
    fn projected_workflow_resolution_accepts_id_alias_and_unique_prefix() {
        let session = session_with_workflows();

        assert_eq!(
            projected_resolve_workflow(&session, "workflow-alpha")
                .expect("resolve by id")
                .id(),
            "workflow-alpha"
        );
        assert_eq!(
            projected_resolve_workflow(&session, "deploy")
                .expect("resolve by alias")
                .id(),
            "workflow-alpha"
        );
        assert_eq!(
            projected_resolve_workflow(&session, "deb")
                .expect("resolve by alias prefix")
                .id(),
            "workflow-beta"
        );
    }

    #[test]
    fn projected_workflow_id_handles_optional_refs() {
        let session = session_with_workflows();

        assert_eq!(
            projected_workflow_id(&session, Some("deploy")).expect("workflow id"),
            Some("workflow-alpha".to_string())
        );
        assert_eq!(
            projected_workflow_id(&session, None).expect("no workflow ref"),
            None
        );
    }

    #[test]
    fn projected_workflow_resolution_rejects_ambiguous_prefixes() {
        let session = session_with_workflows();
        let error = projected_resolve_workflow(&session, "workflow")
            .expect_err("ambiguous workflow prefix should fail");

        assert!(matches!(error, DaemonError::WorkflowNotFound { .. }));
    }

    #[test]
    fn projected_workflow_run_resolution_accepts_id_and_unique_prefix() {
        let session = session_with_workflows();

        assert_eq!(
            projected_resolve_workflow_run(&session, "run-alpha")
                .expect("resolve run by id")
                .id(),
            "run-alpha"
        );
        assert_eq!(
            projected_resolve_workflow_run(&session, "run-b")
                .expect("resolve run by prefix")
                .id(),
            "run-beta"
        );
    }
}
