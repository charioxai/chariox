//! The owner's acquisition: the owner connection and an agent acting for the owner through an access
//! grant take the path the agent's `chariox.machines.*` tools take.
use super::tool_tests::{agent_kernel, service};
use super::*;
use crate::local::{KernelConnectionClass, LocalDaemonResponse};
use crate::runtime::command::{KernelCaller, KernelCallerKind, KernelCommand, KernelCommandSource};

fn option(class_id: &str, region: &str, available: bool) -> crate::local::MachineOption {
    super::tests::option(class_id, region, available)
}

/// A acquisition request as the owner's own connection, or as a grant holder, sends it.
fn rental_command(request: &MachineAcquisitionRequest, grant: Option<&str>) -> KernelCommand {
    let caller = match grant {
        Some(grant) => KernelCaller {
            caller_id: grant.into(),
            caller_kind: KernelCallerKind::LocalClient,
            connection_class: Some(KernelConnectionClass::ExternalAgent),
            ..KernelCaller::default()
        },
        None => KernelCaller {
            caller_kind: KernelCallerKind::LocalClient,
            connection_class: Some(KernelConnectionClass::Terminal),
            ..KernelCaller::default()
        },
    };
    KernelCommand::from_local_request_with_caller(
        "acquisition",
        KernelCommandSource::LocalIpc,
        caller,
        None,
        None,
        &LocalDaemonRequest::MachineAcquisition(request.clone()),
    )
}

async fn acquisition(
    state: &KernelRuntimeState,
    request: MachineAcquisitionRequest,
    grant: Option<&str>,
) -> Result<MachineAcquisitionResult, DaemonError> {
    let command = rental_command(&request, grant);
    let router = state.job_router.as_ref().unwrap();
    match crate::runtime::control_plane::execute_acquire(
        &state.owned.config_projection.snapshot(),
        state,
        &command,
        router.kernel_gateway(),
        &request,
    )
    .await?
    {
        LocalDaemonResponse::MachineAcquisition { result } => Ok(result),
        other => panic!("unexpected response {other:?}"),
    }
}

fn rent(
    request_id: &str,
    class_id: Option<&str>,
    region: Option<&str>,
) -> MachineAcquisitionRequest {
    MachineAcquisitionRequest::Acquire {
        request_id: Some(request_id.into()),
        class_id: class_id.map(str::to_owned),
        region: region.map(str::to_owned),
        timeout_s: None,
        purpose: None,
    }
}

fn allocation_ref(result: &MachineAcquisitionResult) -> String {
    match result {
        MachineAcquisitionResult::Acquired {
            allocation: crate::local::ControlPlaneResponse::Machine { allocation_ref, .. },
            ..
        }
        | MachineAcquisitionResult::Machine(crate::local::ControlPlaneResponse::Machine {
            allocation_ref,
            ..
        }) => allocation_ref.clone(),
        other => panic!("no allocation in {other:?}"),
    }
}

#[tokio::test]
async fn the_owner_lists_options_rents_reads_and_releases_one_machine() {
    let service = service(
        vec![
            option("big", "north", false),
            option("small", "south", true),
        ],
        &[],
    )
    .await;
    let (state, _token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let MachineAcquisitionResult::Options { options, .. } =
        acquisition(&state, MachineAcquisitionRequest::Options {}, None)
            .await
            .unwrap()
    else {
        panic!("options expected")
    };
    assert_eq!(options.len(), 2);
    let rented = acquisition(&state, rent("r1", Some("small"), Some("south")), None)
        .await
        .unwrap();
    let reference = allocation_ref(&rented);
    // A retry with the same request id returns the machine the first try got.
    let again = acquisition(&state, rent("r1", Some("small"), Some("south")), None)
        .await
        .unwrap();
    assert_eq!(allocation_ref(&again), reference);
    let status = acquisition(
        &state,
        MachineAcquisitionRequest::Status {
            allocation_ref: Some(reference.clone()),
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(allocation_ref(&status), reference);
    let released = acquisition(
        &state,
        MachineAcquisitionRequest::Release {
            allocation_ref: reference.clone(),
        },
        None,
    )
    .await
    .unwrap();
    assert!(matches!(
        released,
        MachineAcquisitionResult::Machine(crate::local::ControlPlaneResponse::Machine { ref state, .. }) if state == "deleted"
    ));
}

#[tokio::test]
async fn the_owner_without_a_class_gets_the_first_option_in_stock_like_the_agent() {
    let service = service(
        vec![
            option("big", "north", false),
            option("small", "south", true),
        ],
        &[],
    )
    .await;
    let (state, _token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let rented = acquisition(&state, rent("r1", None, None), None)
        .await
        .unwrap();
    let MachineAcquisitionResult::Acquired {
        chosen: MachineAcquisitionChoice::Option {
            option: Some(option),
        },
        ..
    } = rented
    else {
        panic!("rented option expected")
    };
    assert_eq!(option.class_id, "small");
}

#[tokio::test]
async fn the_owner_is_told_plainly_when_an_option_is_out_of_stock() {
    let service = service(
        vec![option("big", "north", true), option("small", "south", true)],
        &["big"],
    )
    .await;
    let (state, _token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let error = acquisition(&state, rent("r1", Some("big"), Some("north")), None)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("big in north is out of stock"), "{error}");
    assert!(error.contains("In stock now: small in south."), "{error}");
}

#[tokio::test]
async fn an_agent_with_a_grant_and_no_room_raises_the_owners_card_and_rents_after_approval() {
    let service = service(vec![option("small", "south", true)], &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    // The owner asks to approve agent acquisitions.
    let mut config = state.owned.config_projection.snapshot();
    config.user_config.agents.offload_without_asking = Some(false);
    state.owned.config_projection.update(config);
    let runs = state
        .owned
        .provider_store
        .get_runs_by_runtime_mcp_auth_token(&token);
    let grant = state.insert_access_grant_for_test(runs[0].session_id());
    let asking = state.clone();
    let rented = tokio::spawn(async move {
        acquisition(
            &asking,
            rent("r1", Some("small"), Some("south")),
            Some(&grant),
        )
        .await
    });
    let card = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(card) = state
                .owned
                .user_domain_interactions(&state.kernel_owner_user_id())
                .into_iter()
                .find(|card| card.id().starts_with("machine-acquisition-"))
            {
                break card;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the owner's card");
    // Nothing was asked of the service before the owner decided.
    assert!(service.acquires.lock().unwrap().is_empty());
    state
        .with_interaction_answerer(&KernelCaller {
            caller_kind: KernelCallerKind::LocalClient,
            connection_class: Some(KernelConnectionClass::Terminal),
            ..KernelCaller::default()
        })
        .answer_terminal_runtime_interaction(
            "",
            card.id(),
            "approve",
            None,
            Some(&state.kernel_owner_user_id()),
            None,
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    let rented = rented.await.unwrap().unwrap();
    assert_eq!(allocation_ref(&rented), "allocation-1");
    assert_eq!(service.acquires.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_owner_connection_needs_no_card_even_when_agent_rentals_ask() {
    let service = service(vec![option("small", "south", true)], &[]).await;
    let (state, _token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let mut config = state.owned.config_projection.snapshot();
    config.user_config.agents.offload_without_asking = Some(false);
    state.owned.config_projection.update(config);
    let rented = acquisition(&state, rent("r1", Some("small"), Some("south")), None)
        .await
        .unwrap();
    assert_eq!(allocation_ref(&rented), "allocation-1");
}

#[tokio::test]
async fn an_unknown_allocation_is_refused_in_plain_words() {
    let service = service(vec![option("small", "south", true)], &[]).await;
    let (state, _token, _workspace) = agent_kernel(service.api_url.clone()).await;
    for request in [
        MachineAcquisitionRequest::Status {
            allocation_ref: Some("unknown-machine".into()),
        },
        MachineAcquisitionRequest::Release {
            allocation_ref: "unknown-machine".into(),
        },
    ] {
        let error = acquisition(&state, request, None)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("No machine unknown-machine belongs to this account"),
            "{error}"
        );
        assert!(!error.contains("cloud relay request"), "{error}");
    }
}
