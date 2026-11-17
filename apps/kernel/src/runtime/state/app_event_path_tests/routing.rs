//! Protocol 1010: an App with `eventRouting` configures the path for itself.
//! The fixture App stands in for the GitHub App (its manifest declares the
//! generator through `connections`; the seam's own declaration is passed as the
//! scope the broker builds from the signed manifest). The generator, event
//! delivery service and kernel are the real ones of the other event-path drills.
use super::*;
use crate::runtime::state::{Acting, RouteSpec, RoutingError, RoutingScope};
use chariox_app_package::EventRouting;

struct Fixture {
    services: Option<Services>,
    config: crate::config::DaemonConfig,
    router: CommandRouter,
    sessions: Vec<(String, String)>,
    agent: String,
    _cleanup: WorkerCleanup,
    _scratch: Scratch,
}

impl Fixture {
    async fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("chariox-route-{:032x}", rand::random::<u128>()));
        std::fs::create_dir(&root).unwrap();
        let scratch = Scratch(root.clone());
        let services = Services::new(&root).await;
        let config = test_config(&root, &services);
        let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
        let cleanup = WorkerCleanup(app.app_control_service().lifecycle().clone());
        let store = app.durable_state_store();
        install_fixture_app(&store);
        let sessions = vec![workflow(&mut app, &root.join("workspace/first"), "first")];
        let agent = app.agents().get_session_agents(&sessions[0].0)[0]
            .id()
            .to_owned();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        record_fixture_connection(&router.runtime_state(), &config);
        Self {
            services: Some(services),
            config,
            router,
            sessions,
            agent,
            _cleanup: cleanup,
            _scratch: scratch,
        }
    }

    fn state(&self) -> KernelRuntimeState {
        self.router.runtime_state()
    }

    async fn request(&self, request: LocalDaemonRequest) -> LocalDaemonResponse {
        self.router
            .dispatch(
                KernelCommand::from_local_request(
                    format!("req-{}", rand::random::<u64>()),
                    None,
                    None,
                    &request,
                ),
                request,
            )
            .await
            .unwrap()
    }

    async fn grant(&self) {
        let response = self
            .request(LocalDaemonRequest::GrantAppConnection(
                crate::local::GrantAppConnectionRequest {
                    installation_id: "installed".into(),
                    generator_id: GENERATOR.into(),
                    connection_id: "fixture-connection".into(),
                },
            ))
            .await;
        assert!(matches!(
            response,
            LocalDaemonResponse::AppConnections { .. }
        ));
    }

    /// The claims the kernel would send AEDS: one per route that is a source.
    fn claims(&self) -> Vec<(String, bool)> {
        let kernel_id =
            crate::transport::gateway_identity::GatewayIdentity::from_config(&self.config)
                .kernel_id();
        self.state()
            .event_delivery_resumes(&kernel_id, &self.config.event_delivery_environment_id)
            .unwrap()[0]
            .routes
            .iter()
            .map(|route| (route.endpoint_id.clone(), route.active))
            .collect()
    }

    async fn stop(mut self) {
        self.services.take().unwrap().stop().await;
    }
}

fn scope(max_routes: u32) -> RoutingScope {
    RoutingScope {
        owner: "local".into(),
        installation: "installed".into(),
        declared: EventRouting {
            generator: GENERATOR.into(),
            incoming: vec!["received".into()],
            max_routes,
        },
    }
}

fn spec(route_id: &str, repository: &str) -> RouteSpec {
    RouteSpec {
        route_id: route_id.into(),
        event: "received".into(),
        repository: repository.into(),
        source_event_type: "dummy.test".into(),
        source_event_version: 1,
    }
}

#[test]
fn routes_are_the_apps_own_bounded_by_its_grant_and_manifest() {
    on_runtime(routes);
}

async fn routes() {
    let fixture = Fixture::new().await;
    let state = fixture.state();
    let client = crate::runtime::event_catalog_control::AegsManagementHttpClient::default();
    let scope = scope(2);

    // Nothing is routable before the owner grants a connection.
    assert_eq!(
        state
            .app_routing_ensure_route(&client, &scope, spec("r1", "default"))
            .await,
        Err(RoutingError::ConnectionNotGranted)
    );
    let status = state.app_routing_connection_status(&scope).await.unwrap();
    assert_eq!(
        (status.status.as_str(), status.connection_id, status.routes),
        ("not_granted", None, 0)
    );

    fixture.grant().await;
    let status = state.app_routing_connection_status(&scope).await.unwrap();
    assert_eq!(
        (
            status.status.as_str(),
            status.connection_id.as_deref(),
            status.max_routes
        ),
        ("ready", Some("fixture-connection"), 2)
    );

    // A route takes its scope from the generator's own list.
    let first = state
        .app_routing_ensure_route(&client, &scope, spec("r1", "default"))
        .await
        .unwrap();
    assert_eq!((first.repository.as_str(), first.active), ("default", true));
    assert_eq!(fixture.claims(), vec![("r1".to_owned(), true)]);
    // Ensuring again answers the same route; another shape under the id is refused.
    assert_eq!(
        state
            .app_routing_ensure_route(&client, &scope, spec("r1", "default"))
            .await
            .unwrap()
            .route_id,
        "r1"
    );
    let mut other = spec("r1", "default");
    other.source_event_version = 2;
    assert_eq!(
        state.app_routing_ensure_route(&client, &scope, other).await,
        Err(RoutingError::RouteExists)
    );

    // Refused before anything is stored: an event the manifest does not
    // declare, a repository outside the connection, a bad spec.
    let mut undeclared = spec("r2", "default");
    undeclared.event = "changed".into();
    assert_eq!(
        state
            .app_routing_ensure_route(&client, &scope, undeclared)
            .await,
        Err(RoutingError::EventNotDeclared)
    );
    assert_eq!(
        state
            .app_routing_ensure_route(&client, &scope, spec("r2", "elsewhere"))
            .await,
        Err(RoutingError::RepositoryNotInConnection)
    );
    assert_eq!(
        state
            .app_routing_ensure_route(&client, &scope, spec("", "default"))
            .await,
        Err(RoutingError::InvalidArgument)
    );
    assert_eq!(fixture.claims().len(), 1);

    // The owner's hand-made route is neither the App's to change nor counted
    // against its limit.
    assert!(matches!(
        route(&fixture.router, "manual", Value::Null).await.unwrap(),
        LocalDaemonResponse::AppInboxRoutes { .. }
    ));
    assert_eq!(
        state.app_routing_list_routes(&scope).await.unwrap().len(),
        1
    );
    assert_eq!(
        state.app_routing_remove_route(&scope, "manual").await,
        Err(RoutingError::RouteNotFound)
    );
    assert_eq!(
        state
            .app_routing_set_route_active(&scope, "manual", false)
            .await,
        Err(RoutingError::RouteNotFound)
    );

    // Two routes fill the manifest's limit; a third is refused.
    state
        .app_routing_ensure_route(&client, &scope, spec("r2", "second"))
        .await
        .unwrap();
    assert_eq!(
        state
            .app_routing_ensure_route(&client, &scope, spec("r3", "default"))
            .await,
        Err(RoutingError::RouteLimit)
    );

    // Pause: claimed inactive and takes nothing; resume takes occurrences again.
    let paused = state
        .app_routing_set_route_active(&scope, "r1", false)
        .await
        .unwrap();
    assert!(!paused.active);
    assert!(fixture.claims().contains(&("r1".to_owned(), false)));
    assert!(state
        .accept_app_inbox_occurrence("local", "installed", "r1", "occ-1", json!({"text":"x"}))
        .await
        .is_err());
    let resumed = state
        .app_routing_set_route_active(&scope, "r1", true)
        .await
        .unwrap();
    assert!(resumed.active);
    assert!(fixture.claims().contains(&("r1".to_owned(), true)));
    assert!(state
        .accept_app_inbox_occurrence("local", "installed", "r1", "occ-2", json!({"text":"x"}))
        .await
        .is_ok());

    // Removing frees the slot.
    state.app_routing_remove_route(&scope, "r2").await.unwrap();
    assert_eq!(
        state
            .app_routing_ensure_route(&client, &scope, spec("r3", "second"))
            .await
            .unwrap()
            .route_id,
        "r3"
    );

    // Revoking the grant ends the App's routes and leaves the owner's.
    let response = fixture
        .request(LocalDaemonRequest::RevokeAppConnection(
            crate::local::RevokeAppConnectionRequest {
                installation_id: "installed".into(),
                connection_id: "fixture-connection".into(),
            },
        ))
        .await;
    assert!(matches!(
        response,
        LocalDaemonResponse::AppConnections { .. }
    ));
    assert!(state
        .app_routing_list_routes(&scope)
        .await
        .unwrap()
        .is_empty());
    let left: Vec<_> = fixture.claims().into_iter().map(|claim| claim.0).collect();
    assert_eq!(left, vec!["manual".to_owned()]);
    assert_eq!(
        state
            .app_routing_ensure_route(&client, &scope, spec("r1", "default"))
            .await,
        Err(RoutingError::ConnectionNotGranted)
    );
    fixture.stop().await;
}

#[test]
fn authorization_and_resources_come_from_the_granted_connection_only() {
    on_runtime(authorization);
}

async fn authorization() {
    let fixture = Fixture::new().await;
    let state = fixture.state();
    let client = crate::runtime::event_catalog_control::AegsManagementHttpClient::default();
    let scope = scope(5);
    assert_eq!(
        state
            .app_routing_connection_resources(&client, &scope, None, None, 10)
            .await
            .unwrap_err(),
        RoutingError::ConnectionNotGranted
    );
    let (id, url) = state
        .app_routing_start_authorization(&client, &scope, None)
        .await
        .map(|(_, url)| ("started", url))
        .unwrap();
    assert_eq!(id, "started");
    assert!(url.starts_with("https://provider.example/consent"));
    fixture.grant().await;
    let page = state
        .app_routing_connection_resources(&client, &scope, Some("alpha".into()), None, 10)
        .await
        .unwrap();
    assert_eq!(
        page.resources
            .iter()
            .map(|resource| resource.connection_scope.as_str())
            .collect::<Vec<_>>(),
        ["default", "second"]
    );
    fixture.stop().await;
}

#[test]
fn an_agent_binds_only_itself_and_a_person_chooses_workflows() {
    on_runtime(targets);
}

async fn targets() {
    let fixture = Fixture::new().await;
    let state = fixture.state();
    let scope = scope(5);
    let agent = fixture.agent.clone();
    let (session, publication) = fixture.sessions[0].clone();

    // The caller record is the kernel's: two agents at once name nobody, and a
    // person's call that overlaps an agent's gives the agent no more.
    let agent_call = json!({"kind":"agent","id":agent});
    let person_call = json!({"kind":"human","id":"local"});
    assert_eq!(
        Acting::of(std::slice::from_ref(&agent_call)),
        Ok(Acting::Agent(agent.clone()))
    );
    assert_eq!(
        Acting::of(std::slice::from_ref(&person_call)),
        Ok(Acting::Person)
    );
    assert_eq!(
        Acting::of(&[person_call.clone(), agent_call.clone()]),
        Ok(Acting::Agent(agent.clone()))
    );
    assert_eq!(
        Acting::of(&[agent_call.clone(), json!({"kind":"agent","id":"other"})]),
        Err(RoutingError::NotAllowed)
    );
    assert_eq!(Acting::of(&[]), Err(RoutingError::NotAllowed));
    assert_eq!(
        Acting::of(&[json!({"kind":"background","id":"wake"})]),
        Err(RoutingError::NotAllowed)
    );

    let me = Acting::Agent(agent.clone());
    let person = Acting::Person;
    // An agent binds itself, once; another agent or an undeclared event is refused.
    let bound = state
        .app_routing_bind_agent(&scope, &me, "changed".into(), None)
        .await
        .unwrap();
    assert_eq!(
        (bound.kind, bound.agent_id.as_deref()),
        ("agent", Some(agent.as_str()))
    );
    assert_eq!(
        state
            .app_routing_bind_agent(&scope, &me, "changed".into(), Some(agent.clone()))
            .await
            .unwrap()
            .automation_id,
        bound.automation_id
    );
    assert_eq!(
        state
            .app_routing_bind_agent(&scope, &me, "changed".into(), Some("someone-else".into()))
            .await,
        Err(RoutingError::NotAllowed)
    );
    assert_eq!(
        state
            .app_routing_bind_agent(&scope, &me, "unknown_event".into(), None)
            .await,
        Err(RoutingError::EventNotDeclared)
    );
    assert_eq!(
        state
            .app_routing_bind_agent(&scope, &person, "changed".into(), None)
            .await,
        Err(RoutingError::InvalidArgument)
    );
    // A person may name the agent.
    assert_eq!(
        state
            .app_routing_bind_agent(&scope, &person, "changed".into(), Some(agent.clone()))
            .await
            .unwrap()
            .automation_id,
        bound.automation_id
    );

    // A workflow target is a person's choice, never an agent's.
    assert_eq!(
        state
            .app_routing_bind_workflow(
                &scope,
                &me,
                "changed".into(),
                session.clone(),
                publication.clone(),
                None
            )
            .await,
        Err(RoutingError::NotAllowed)
    );
    let workflow = state
        .app_routing_bind_workflow(
            &scope,
            &person,
            "changed".into(),
            session.clone(),
            publication.clone(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(workflow.kind, "workflow");
    let listed = state.app_routing_list_targets(&scope).await.unwrap();
    assert_eq!(listed.len(), 2);

    // An agent ends only its own target; a person ends any.
    assert_eq!(
        state
            .app_routing_unbind_target(&scope, &me, &workflow.automation_id)
            .await,
        Err(RoutingError::NotAllowed)
    );
    assert_eq!(
        state
            .app_routing_unbind_target(
                &scope,
                &Acting::Agent("someone-else".into()),
                &bound.automation_id
            )
            .await,
        Err(RoutingError::NotAllowed)
    );
    state
        .app_routing_unbind_target(&scope, &me, &bound.automation_id)
        .await
        .unwrap();
    state
        .app_routing_unbind_target(&scope, &person, &workflow.automation_id)
        .await
        .unwrap();
    assert!(state
        .app_routing_list_targets(&scope)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        state
            .app_routing_unbind_target(&scope, &person, &workflow.automation_id)
            .await,
        Err(RoutingError::TargetNotFound)
    );
    fixture.stop().await;
}
