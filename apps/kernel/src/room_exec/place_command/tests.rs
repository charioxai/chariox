use super::*;
use crate::local::GatewayKernelVia;
use crate::room_exec::queue_tests::{limited, queued, TempDir};

fn held(
    service: &RoomExecService,
    dir: &std::path::Path,
    script: &str,
    agent: Option<&str>,
) -> String {
    service
        .open_as(
            "room",
            crate::session::DEFAULT_LOCAL_USER_ID,
            dir,
            &["/bin/bash".into(), "-c".into(), script.into()],
            None,
            agent.map(str::to_owned),
        )
        .unwrap()
}

fn kernel(name: &str, id: &str, status: GatewayKernelStatus, gateway_ready: bool) -> GatewayKernel {
    GatewayKernel {
        name: name.into(),
        kernel_id: id.into(),
        label: name.into(),
        machine_id: format!("machine-{id}"),
        machine_alias: None,
        status,
        last_seen_ms: 1,
        gateway_ready,
        via: GatewayKernelVia::Hosted,
    }
}

#[test]
fn only_the_owners_other_online_gateway_kernels_are_destinations() {
    let listed = vec![
        kernel("b3", "k-b3", GatewayKernelStatus::Online, true),
        kernel("here", "k-here", GatewayKernelStatus::Online, true),
        kernel("away", "k-away", GatewayKernelStatus::Offline, true),
        kernel("lost", "k-lost", GatewayKernelStatus::Unpaired, true),
        kernel("old", "k-old", GatewayKernelStatus::Online, false),
        kernel("mac/dev", "k-mac", GatewayKernelStatus::Online, true),
    ];
    assert_eq!(destinations(listed, "k-here"), vec!["b3", "mac/dev"]);
}

#[tokio::test]
async fn a_held_launch_lists_the_kernels_its_requester_may_place_it_on() {
    let dir = TempDir::new();
    let service = limited(0, u64::MAX / 2);
    let by_agent = held(&service, dir.path(), "exit 0", Some("agent-1"));
    let by_person = held(&service, dir.path(), "exit 0", None);
    queued(&service, &by_agent).await;
    queued(&service, &by_person).await;
    let kernels = vec!["b3".to_string()];
    let offered = |requester: &Requester| {
        let mut execs = service.list("room");
        offer_placement(&mut execs, requester, &kernels);
        execs
            .into_iter()
            .map(|exec| (exec.exec_id.clone(), exec.placeable_on))
            .collect::<std::collections::BTreeMap<_, _>>()
    };

    let person = offered(&Requester::Person);
    assert_eq!(person[&by_agent], kernels);
    assert_eq!(person[&by_person], kernels);
    let agent = offered(&Requester::Agent("agent-1".into()));
    assert_eq!(agent[&by_agent], kernels);
    assert!(
        agent[&by_person].is_empty(),
        "an agent is offered only its own launches"
    );
    let other = offered(&Requester::Agent("agent-2".into()));
    assert!(other.values().all(Vec::is_empty));

    // The field is optional on the wire: a row without an offer does not carry it.
    let mut execs = service.list("room");
    assert!(!serde_json::to_string(&execs[0])
        .unwrap()
        .contains("placeable_on"));
    offer_placement(&mut execs, &Requester::Person, &kernels);
    assert!(serde_json::to_string(&execs[0])
        .unwrap()
        .contains("\"placeable_on\":[\"b3\"]"));
}

#[tokio::test]
async fn a_launch_that_started_is_not_offered() {
    let dir = TempDir::new();
    let service = limited(u64::MAX / 2, u64::MAX / 2);
    let id = held(&service, dir.path(), "sleep 30", Some("agent-1"));
    let mut execs = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let execs = service.list("room");
            if execs
                .iter()
                .any(|exec| exec.exec_id == id && matches!(exec.state, ExecState::Running))
            {
                return execs;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    offer_placement(&mut execs, &Requester::Person, &["b3".to_string()]);
    assert!(execs[0].placeable_on.is_empty());
    service.cancel("room", &id).unwrap();
}
