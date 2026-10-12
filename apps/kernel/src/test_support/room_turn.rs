//! MP-08/MP-09/MP-10/MP-11 A02: admit causal turns for supplementary room fixtures.
use super::*;
pub(crate) fn admit_room_test_turn(app: &mut DaemonApp, room: &str, agent: &str) {
    if app
        .sessions()
        .get_session(room)
        .unwrap()
        .active_prompt_for_agent(agent)
        .is_some()
    {
        return;
    }
    if app.providers().get_run_for_agent(room, agent).is_none() {
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    room,
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "room-model",
                )
                .with_agent_id(agent),
            )
            .unwrap();
        app.update_provider_run_projection(run);
    }
    // The fixture turn stands in for the room owner's own request.
    let owner = app
        .sessions()
        .get_session(room)
        .unwrap()
        .owner_user_id()
        .to_string();
    let attachment = crate::app::KernelSessionService::new(app)
        .attach(crate::attachment::AttachRequest::for_user(
            room,
            format!("room-fixture:{agent}"),
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            owner,
        ))
        .unwrap();
    app.submit_prompt(
        room,
        attachment.id(),
        Some(agent),
        "MP-08/MP-09/MP-10/MP-11: exercise scoped room work",
        Vec::new(),
    )
    .unwrap();
    assert!(app
        .sessions()
        .get_session(room)
        .unwrap()
        .active_prompt_for_agent(agent)
        .is_some());
}
