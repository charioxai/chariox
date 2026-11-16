//! Who answers for a workflow: the creating agent by default, the person otherwise, and the
//! reach rules for naming someone else, through the local requests and the room command.
use super::*;
use crate::local::{SetWorkflowResponsibleRequest, WorkflowResponsibleTarget};
use crate::session::{CollaborationLevel, WorkflowResponsible};

async fn room_command(
    router: &CommandRouter,
    auth: &str,
    command: &str,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    router
        .dispatch_authenticated_runtime_tool_call(
            auth,
            "chariox.room.run_command",
            serde_json::json!({"command":command}),
        )
        .await
        .unwrap()
}

struct Responsible {
    _env: TestMetaRuntimeEnv,
    app: Arc<Mutex<DaemonApp>>,
    router: CommandRouter,
    kernel: String,
    room: String,
    other_room: String,
    a: crate::agent::AgentInstance,
    peer: crate::agent::AgentInstance,
    guest: crate::agent::AgentInstance,
    elsewhere: crate::agent::AgentInstance,
    auth_a: String,
    auth_peer: String,
    auth_guest: String,
}

impl Responsible {
    /// A Room with an agent `a`, a peer, a guest's agent, and another Room of the same owner.
    async fn new(label: &str) -> Self {
        let env = TestMetaRuntimeEnv::new(label);
        let workspace = env.root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let mut config = DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let (session, a) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                workspace.to_string_lossy(),
                workspace.to_string_lossy(),
            ))
            .unwrap();
        let (other, elsewhere) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                workspace.to_string_lossy(),
                workspace.to_string_lossy(),
            ))
            .unwrap();
        let peer = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("peer"))
            .unwrap();
        let mut room = app.sessions().get_session(session.id()).unwrap();
        room.add_member(
            "guest-user",
            Some(room.owner_user_id().to_owned()),
            CollaborationLevel::Full,
        );
        app.sessions_mut().restore_session(room);
        let guest = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("guest")
                    .with_owner_user_id("guest-user"),
            )
            .unwrap();
        let mut token = |agent: &crate::agent::AgentInstance, room: &str| {
            launch_test_provider(&mut app, room, agent.id(), "dev-stub", "dev-stub", "m")
                .runtime_mcp_auth_token()
                .unwrap()
                .to_string()
        };
        let auth_a = token(&a, session.id());
        let auth_peer = token(&peer, session.id());
        let auth_guest = token(&guest, session.id());
        let kernel = session.host_daemon_id().to_owned();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
        Self {
            _env: env,
            app,
            router,
            kernel,
            room: session.id().to_owned(),
            other_room: other.id().to_owned(),
            a,
            peer,
            guest,
            elsewhere,
            auth_a,
            auth_peer,
            auth_guest,
        }
    }

    async fn person(
        &self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.router
            .dispatch(
                KernelCommand::from_local_request("workflow-responsible", None, None, &request),
                request,
            )
            .await
    }

    async fn person_creates(&self, alias: &str) -> String {
        match self
            .person(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: self.room.clone(),
                alias: Some(alias.into()),
            }))
            .await
            .unwrap()
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow.id().to_owned(),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    async fn person_sets(
        &self,
        workflow: &str,
        responsible: WorkflowResponsibleTarget,
    ) -> Result<crate::session::WorkflowDefinition, DaemonError> {
        match self
            .person(LocalDaemonRequest::SetWorkflowResponsible(
                SetWorkflowResponsibleRequest {
                    session_id: self.room.clone(),
                    workflow_ref: workflow.into(),
                    responsible,
                },
            ))
            .await?
        {
            LocalDaemonResponse::WorkflowResponsibleSet { workflow, .. } => Ok(workflow),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    fn target(&self, room: &str, agent: &crate::agent::AgentInstance) -> WorkflowResponsibleTarget {
        WorkflowResponsibleTarget::Agent {
            kernel_id: self.kernel.clone(),
            room_id: room.into(),
            agent_id: agent.id().into(),
        }
    }

    async fn responsible(&self, workflow: &str) -> WorkflowResponsible {
        let app = self.app.lock().await;
        app.sessions()
            .resolve_workflow_ref(&self.room, workflow)
            .unwrap()
            .responsible()
            .clone()
    }

    /// The agents holding a responsible trigger, in every Room.
    async fn responsible_triggers(&self) -> Vec<String> {
        let app = self.app.lock().await;
        let mut agents = [&self.room, &self.other_room]
            .into_iter()
            .flat_map(|room| {
                app.sessions()
                    .get_session(room)
                    .unwrap()
                    .agent_triggers()
                    .iter()
                    .filter(|trigger| trigger.responsible_for().is_some())
                    .map(|trigger| trigger.agent_id().to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        agents.sort();
        agents
    }

    fn agent_label(agent: &crate::agent::AgentInstance) -> String {
        agent.alias().unwrap_or(agent.agent_ref()).to_owned()
    }
}

#[test]
fn a_workflow_starts_with_its_creating_agent_or_the_person() {
    run_large_stack_async_test("wf-responsible-default", || async {
        let t = Responsible::new("wf-responsible-default").await;
        let person = t.person_creates("by-person").await;
        assert_eq!(t.responsible(&person).await, WorkflowResponsible::Person);
        assert!(t.responsible_triggers().await.is_empty());

        assert!(
            room_command(&t.router, &t.auth_a, "workflow new by-agent")
                .await
                .ok
        );
        assert_eq!(
            t.responsible("by-agent").await,
            WorkflowResponsible::Agent {
                kernel_id: t.kernel.clone(),
                room_id: t.room.clone(),
                agent_id: t.a.id().into(),
                label: Responsible::agent_label(&t.a),
            }
        );
        assert_eq!(t.responsible_triggers().await, vec![t.a.id().to_owned()]);

        // The kernel keeps that trigger: it is not among the agent's own, and the agent cannot
        // remove it.
        let own = room_command(&t.router, &t.auth_a, "trigger list").await;
        assert_eq!(own.payload["response"]["triggers"], serde_json::json!([]));
        let trigger_id = {
            let app = t.app.lock().await;
            let room = app.sessions().get_session(&t.room).unwrap();
            room.agent_triggers()[0].id().to_owned()
        };
        let removed = room_command(
            &t.router,
            &t.auth_a,
            &format!("trigger remove {trigger_id}"),
        )
        .await;
        assert!(!removed.ok, "{removed:?}");
        assert_eq!(t.responsible_triggers().await, vec![t.a.id().to_owned()]);

        // The responsible is readable through the room command.
        let read = room_command(&t.router, &t.auth_peer, "workflow responsible by-agent").await;
        assert!(read.ok, "{read:?}");
        assert_eq!(
            read.payload["response"]["workflow"]["responsible"]["agent_id"],
            t.a.id()
        );
    });
}

#[test]
fn a_sub_agent_that_creates_a_workflow_leaves_it_to_the_person() {
    run_large_stack_async_test("wf-responsible-sub", || async {
        let t = Responsible::new("wf-responsible-sub").await;
        let spawned =
            room_command(&t.router, &t.auth_a, "agent spawn kid --provider dev-stub").await;
        assert!(spawned.ok, "{spawned:?}");
        let auth_kid = {
            let mut app = t.app.lock().await;
            let kid = app
                .agents()
                .get_session_agents(&t.room)
                .into_iter()
                .find(|agent| agent.alias() == Some("kid"))
                .unwrap();
            launch_test_provider(&mut app, &t.room, kid.id(), "dev-stub", "dev-stub", "m")
                .runtime_mcp_auth_token()
                .unwrap()
                .to_string()
        };
        assert!(
            room_command(&t.router, &auth_kid, "workflow new by-kid")
                .await
                .ok
        );
        assert_eq!(t.responsible("by-kid").await, WorkflowResponsible::Person);
        // The creator's ancestor may name someone: it owns what its descendants made.
        let named = room_command(&t.router, &t.auth_a, "workflow responsible by-kid peer").await;
        assert!(named.ok, "{named:?}");
        assert_eq!(
            t.responsible_triggers().await,
            vec![t.peer.id().to_owned()],
            "one trigger, the new responsible's"
        );
    });
}

#[test]
fn the_person_names_any_ordinary_agent_of_their_rooms_and_the_trigger_follows() {
    run_large_stack_async_test("wf-responsible-person", || async {
        let t = Responsible::new("wf-responsible-person").await;
        let flow = t.person_creates("flow").await;

        let set = t
            .person_sets(&flow, t.target(&t.room, &t.peer))
            .await
            .unwrap();
        assert_eq!(
            set.responsible(),
            &WorkflowResponsible::Agent {
                kernel_id: t.kernel.clone(),
                room_id: t.room.clone(),
                agent_id: t.peer.id().into(),
                label: Responsible::agent_label(&t.peer),
            }
        );
        assert_eq!(t.responsible_triggers().await, vec![t.peer.id().to_owned()]);

        // Another Room of the owner is as good; the old agent's trigger is replaced.
        t.person_sets(&flow, t.target(&t.other_room, &t.elsewhere))
            .await
            .unwrap();
        assert_eq!(
            t.responsible_triggers().await,
            vec![t.elsewhere.id().to_owned()]
        );

        t.person_sets(&flow, WorkflowResponsibleTarget::Person)
            .await
            .unwrap();
        assert_eq!(t.responsible(&flow).await, WorkflowResponsible::Person);
        assert!(t.responsible_triggers().await.is_empty());
    });
}

#[test]
fn only_an_ordinary_top_level_agent_of_the_owner_can_be_responsible() {
    run_large_stack_async_test("wf-responsible-target", || async {
        let t = Responsible::new("wf-responsible-target").await;
        let flow = t.person_creates("flow").await;
        let spawned =
            room_command(&t.router, &t.auth_a, "agent spawn kid --provider dev-stub").await;
        assert!(spawned.ok, "{spawned:?}");
        let kid = {
            let app = t.app.lock().await;
            app.agents()
                .get_session_agents(&t.room)
                .into_iter()
                .find(|agent| agent.alias() == Some("kid"))
                .unwrap()
        };
        for (what, target) in [
            ("a sub-agent", t.target(&t.room, &kid)),
            ("a guest's agent", t.target(&t.room, &t.guest)),
            (
                "an agent that does not exist",
                WorkflowResponsibleTarget::Agent {
                    kernel_id: t.kernel.clone(),
                    room_id: t.room.clone(),
                    agent_id: "no-such-agent".into(),
                },
            ),
            (
                "a Room that does not exist",
                WorkflowResponsibleTarget::Agent {
                    kernel_id: t.kernel.clone(),
                    room_id: "no-such-room".into(),
                    agent_id: t.peer.id().into(),
                },
            ),
        ] {
            let refused = t.person_sets(&flow, target).await.unwrap_err();
            assert!(
                refused.to_string().contains("pick an ordinary agent"),
                "{what}: {refused}"
            );
        }
        assert_eq!(t.responsible(&flow).await, WorkflowResponsible::Person);
        assert!(t.responsible_triggers().await.is_empty());
    });
}

#[test]
fn an_agent_takes_a_workflow_for_itself_but_names_another_only_as_responsible_or_creator() {
    run_large_stack_async_test("wf-responsible-reach", || async {
        let t = Responsible::new("wf-responsible-reach").await;
        // Made by the person: no agent created it.
        let flow = t.person_creates("flow").await;

        // A stranger may not name a third agent, nor hand the workflow back.
        let refused = room_command(&t.router, &t.auth_a, "workflow responsible flow peer").await;
        assert!(!refused.ok, "{refused:?}");
        let refused = room_command(&t.router, &t.auth_a, "workflow responsible flow you").await;
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(t.responsible(&flow).await, WorkflowResponsible::Person);

        // It may take it for itself.
        let taken = room_command(&t.router, &t.auth_a, "workflow responsible flow me").await;
        assert!(taken.ok, "{taken:?}");
        assert_eq!(t.responsible_triggers().await, vec![t.a.id().to_owned()]);

        // Another stranger still may not take it back for the person or hand it to a third.
        for command in [
            "workflow responsible flow you",
            "workflow responsible flow a",
        ] {
            let refused = room_command(&t.router, &t.auth_peer, command).await;
            assert!(!refused.ok, "{command}: {refused:?}");
        }
        // The current responsible may hand it over.
        let handed = room_command(&t.router, &t.auth_a, "workflow responsible flow peer").await;
        assert!(handed.ok, "{handed:?}");
        assert_eq!(t.responsible_triggers().await, vec![t.peer.id().to_owned()]);
    });
}

#[test]
fn the_creator_may_name_someone_after_it_stopped_being_responsible() {
    run_large_stack_async_test("wf-responsible-creator", || async {
        let t = Responsible::new("wf-responsible-creator").await;
        assert!(
            room_command(&t.router, &t.auth_a, "workflow new flow")
                .await
                .ok
        );
        let taken = room_command(&t.router, &t.auth_peer, "workflow responsible flow me").await;
        assert!(taken.ok, "{taken:?}");
        let back = room_command(&t.router, &t.auth_a, "workflow responsible flow you").await;
        assert!(back.ok, "the creator names the person: {back:?}");
        assert_eq!(t.responsible("flow").await, WorkflowResponsible::Person);
        assert!(t.responsible_triggers().await.is_empty());
    });
}

#[test]
fn a_guest_cannot_change_who_is_responsible() {
    run_large_stack_async_test("wf-responsible-guest", || async {
        let t = Responsible::new("wf-responsible-guest").await;
        let flow = t.person_creates("flow").await;
        let refused = room_command(&t.router, &t.auth_guest, "workflow responsible flow me").await;
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(t.responsible(&flow).await, WorkflowResponsible::Person);
        assert!(t.responsible_triggers().await.is_empty());
    });
}

#[test]
fn a_collaborator_cannot_name_who_is_responsible_either() {
    run_large_stack_async_test("wf-responsible-collaborator", || async {
        let t = Responsible::new("wf-responsible-collaborator").await;
        let flow = t.person_creates("flow").await;
        let request = LocalDaemonRequest::SetWorkflowResponsible(SetWorkflowResponsibleRequest {
            session_id: t.room.clone(),
            workflow_ref: flow.clone(),
            responsible: WorkflowResponsibleTarget::Agent {
                kernel_id: t.kernel.clone(),
                room_id: t.room.clone(),
                agent_id: t.peer.id().into(),
            },
        });
        let refused = t
            .router
            .dispatch(
                remote_command_for_request(&request, Some("guest-user")),
                request,
            )
            .await;
        assert!(refused.is_err(), "{refused:?}");
        assert_eq!(t.responsible(&flow).await, WorkflowResponsible::Person);
    });
}
