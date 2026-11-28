#[cfg(test)]
mod tests {
    use chariox_kernel::session::{CreateSessionRequest, SessionService};
    use chariox_kernel::DaemonConfig;

    #[test]
    fn visible_session_project_assignment_survives_release_builds() {
        let mut service = SessionService::new(&DaemonConfig::for_tests());
        let created = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        assert!(!created.project_id().is_empty());
        assert_eq!(
            service
                .get_project(created.project_id())
                .expect("session project should exist")
                .workspace_id(),
            created.workspace_id()
        );
    }
}
