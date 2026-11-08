//! MP-08 / MP-09 / MP-10 / MP-11: who may run a held launch on another kernel.
use super::tests::{fixture, running};
use crate::error::DaemonError;
use crate::local::{RoomTerminalOperation, RoomTerminalRequest};
use crate::room_exec::Requester;
use crate::runtime::command::{KernelCaller, KernelCommandSource};

fn place(
    session_id: &str,
    acting_agent: Option<&str>,
    host_agent: Option<&str>,
) -> RoomTerminalRequest {
    RoomTerminalRequest {
        session_id: session_id.into(),
        acting_agent: acting_agent.map(str::to_owned),
        host_agent: host_agent.map(str::to_owned),
        operation: RoomTerminalOperation::ExecPlace {
            exec_id: "exec-1".into(),
            kernel: "kernel-b".into(),
            directory: None,
        },
    }
}

fn reach_caller(reach: &crate::runtime::state::reach::ReachCaller) -> KernelCaller {
    let mut caller = KernelCaller::for_source(&KernelCommandSource::LocalIpc);
    caller.caller_id = reach.id.clone();
    caller.user_id = Some(reach.owner_user_id.clone());
    caller
}

fn refused(
    result: Result<super::super::room_exec_place_runtime_state::ExecPlacer, DaemonError>,
) -> String {
    match result {
        Ok(_) => panic!("the request was authorized"),
        Err(error) => error.to_string(),
    }
}

#[tokio::test]
async fn a_person_and_the_launching_agent_are_authorized_and_nobody_else() {
    let f = fixture();
    running(&f);
    let session = f.request.session_id.clone();
    let reach = f.state.reach_for_auth_token("sudo-fixture-bearer").unwrap();

    // The owner's person acts under the owner's grant on this kernel.
    let person = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    let placer = f
        .state
        .authorize_exec_place(&person, &place(&session, None, None))
        .unwrap();
    assert_eq!(placer.requester, Requester::Person);
    assert!(
        placer.origin.grant_id.starts_with("owner:"),
        "{}",
        placer.origin.grant_id
    );
    assert!(
        f.state.gateway_origin(&placer.origin.grant_id).is_some(),
        "the gateway accepts the owner's grant"
    );
    f.state
        .authorize_gateway_request(
            &placer.origin.grant_id,
            &crate::local::LocalDaemonRequest::RoomTerminal(place(&session, None, None)),
        )
        .unwrap();

    // The agent acts under its own live reach.
    let placer = f
        .state
        .authorize_exec_place(&reach_caller(&reach), &place(&session, None, None))
        .unwrap();
    assert_eq!(placer.requester, Requester::Agent(reach.agent_id.clone()));
    assert_eq!(placer.origin.grant_id, reach.id);
    let named = f
        .state
        .authorize_exec_place(
            &reach_caller(&reach),
            &place(&session, Some(&reach.agent_id), None),
        )
        .unwrap();
    assert_eq!(named.requester, Requester::Agent(reach.agent_id.clone()));

    // Not another agent's name, not a name without a live grant, no worker host.
    assert!(refused(f.state.authorize_exec_place(
        &reach_caller(&reach),
        &place(&session, Some("agent-other"), None)
    ))
    .contains("agent-other"));
    refused(
        f.state
            .authorize_exec_place(&person, &place(&session, Some(&reach.agent_id), None)),
    );
    refused(
        f.state
            .authorize_exec_place(&person, &place(&session, None, Some(&reach.agent_id))),
    );
    let mut ended = reach_caller(&reach);
    ended.caller_id = "reach:finished-run".into();
    refused(
        f.state
            .authorize_exec_place(&ended, &place(&session, None, None)),
    );

    // A stranger to the Room and a kernel peer are refused.
    let mut stranger = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    stranger.user_id = Some("someone-else".into());
    refused(
        f.state
            .authorize_exec_place(&stranger, &place(&session, None, None)),
    );
    let peer = KernelCaller::for_source(&KernelCommandSource::RelayPeer);
    refused(
        f.state
            .authorize_exec_place(&peer, &place(&session, None, None)),
    );
}
