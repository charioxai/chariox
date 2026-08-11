use crate::support::kernel_websocket::*;
use arroba_kernel::local::{
    DeleteSessionRequest, ListProjectsRequest, ListSessionsRequest, LocalDaemonRequest,
};
use arroba_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use arroba_kernel::session::CreateSessionRequest;
use arroba_kernel::{DaemonApp, DaemonConfig};
use tokio::sync::oneshot;

#[test]
fn kernel_websocket_keeps_projects_bound_to_visible_sessions() {
    crate::run_kernel_websocket_runtime_test(async {
        let mut config = DaemonConfig::for_tests();
        let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
        config.kernel_websocket_port = kernel_websocket_port;
        config.runtime_mcp_port = unused_tcp_port();
        let app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            run_kernel_websocket_server_on_listener(
                std::sync::Arc::new(tokio::sync::Mutex::new(app)),
                kernel_websocket_listener,
                async {
                    let _ = shutdown_rx.await;
                },
            )
            .await
        });

        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let created = send_request(
            &mut socket,
            "create-visible-session",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-project-lifecycle",
                "worktree-project-lifecycle",
            )),
        )
        .await;
        let session = &response_variant(&created, "SessionCreated")["session"];
        let session_id = session["id"]
            .as_str()
            .expect("created session id should exist")
            .to_string();
        let project_id = session["project_id"]
            .as_str()
            .expect("visible session project id should exist")
            .to_string();
        assert!(!project_id.is_empty());

        let projects = send_request(
            &mut socket,
            "list-projects-with-visible-session",
            LocalDaemonRequest::ListProjects(ListProjectsRequest {
                include_archived: true,
            }),
        )
        .await;
        let projects = response_variant(&projects, "ProjectsListed")["projects"]
            .as_array()
            .expect("projects should be an array");
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["id"].as_str(), Some(project_id.as_str()));

        send_request(
            &mut socket,
            "delete-last-visible-session",
            LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
                session_ref: session_id,
                workspace_id: None,
            }),
        )
        .await;
        let projects = send_request(
            &mut socket,
            "list-projects-after-last-session-delete",
            LocalDaemonRequest::ListProjects(ListProjectsRequest {
                include_archived: true,
            }),
        )
        .await;
        assert!(response_variant(&projects, "ProjectsListed")["projects"]
            .as_array()
            .expect("projects should be an array")
            .is_empty());

        let hidden = send_request(
            &mut socket,
            "create-hidden-runtime",
            LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new(
                    "workspace-publication-runtime",
                    "worktree-publication-runtime",
                )
                .with_hidden(true),
            ),
        )
        .await;
        assert_eq!(
            response_variant(&hidden, "SessionCreated")["session"]["project_id"].as_str(),
            Some("")
        );

        let sessions = send_request(
            &mut socket,
            "list-visible-sessions-after-hidden-runtime",
            LocalDaemonRequest::ListSessions(ListSessionsRequest),
        )
        .await;
        assert!(response_variant(&sessions, "SessionsListed")["sessions"]
            .as_array()
            .expect("sessions should be an array")
            .is_empty());
        let projects = send_request(
            &mut socket,
            "list-projects-after-hidden-runtime",
            LocalDaemonRequest::ListProjects(ListProjectsRequest {
                include_archived: true,
            }),
        )
        .await;
        assert!(response_variant(&projects, "ProjectsListed")["projects"]
            .as_array()
            .expect("projects should be an array")
            .is_empty());

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("kernel websocket task should join")
            .expect("kernel websocket server should shut down cleanly");
    });
}
