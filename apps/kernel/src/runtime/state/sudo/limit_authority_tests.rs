//! Row 569: what lets an agent raise the account's machine limits: the owner's own `/sudo` naming
//! the raise, else a card the person answers. The kernel decides; the agent's words decide nothing.
use super::limit_grant::LimitGrant;
use super::tests::{fixture, running, Fixture};
use super::window_tests::open;
use super::*;
use crate::local::{
    AccountLimitValues, ControlPlaneRequest, KernelConnectionClass, LimitApprovalKind,
    LocalDaemonRequest,
};
use crate::runtime::state::machine_acquisition::limits::AccountLimits;

fn current() -> AccountLimits {
    AccountLimits {
        values: AccountLimitValues {
            machines_at_once: 2,
            spend_per_day_micros: 10_000_000,
        },
        currency: "EUR".into(),
        version: 1,
    }
}

fn set(machines: u32, spend: u64) -> ControlPlaneRequest {
    ControlPlaneRequest::SetLimits {
        expected_version: 1,
        machines_at_once: machines,
        spend_per_day_micros: spend,
        approval: None,
    }
}

/// The sudo fixture's agent asking the control plane.
fn from_the_agent(f: &Fixture, turn: &KernelSudoTurn, request: &ControlPlaneRequest) -> KernelCommand {
    let mut command = KernelCommand::from_local_request(
        "limits",
        None,
        None,
        &LocalDaemonRequest::ControlPlane(request.clone()),
    );
    command.agent_id = Some(turn.agent_id.clone());
    command.session_id = Some(f.request.session_id.clone());
    command.caller.metaagent_id = Some(turn.agent_id.clone());
    command.caller.connection_class = Some(KernelConnectionClass::KernelAgent);
    command
}

/// A running window whose grant is `grant`.
fn granted(f: &Fixture, grant: Option<LimitGrant>) -> KernelSudoTurn {
    let turn = running(f);
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .get_mut(&turn.entry_id)
        .unwrap()
        .limit_grant = grant;
    turn
}

fn machines(n: u32) -> Option<LimitGrant> {
    LimitGrant::parse(&format!("raise limits to {n} machines"))
}

fn no_card(f: &Fixture) {
    assert!(f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap()
        .active_interactions()
        .is_empty());
}

#[tokio::test]
async fn sudo_grant_raises_without_a_card_exactly_as_named() {
    let f = fixture();
    let turn = granted(&f, machines(4));
    for to in [3, 4] {
        let request = set(to, 10_000_000);
        let approval = f
            .state
            .authorize_account_change(&from_the_agent(&f, &turn, &request), &current(), &request)
            .await
            .unwrap()
            .expect("the raise carries the person's approval");
        assert_eq!(approval.kind, LimitApprovalKind::Sudo);
        assert_eq!(approval.reference, turn.entry_id);
        assert_eq!(approval.from, current().values);
        assert_eq!(approval.to.machines_at_once, to);
        no_card(&f);
    }
}

async fn asks_a_card(f: &Fixture, turn: &KernelSudoTurn, request: ControlPlaneRequest) -> String {
    let state = f.state.clone();
    let command = from_the_agent(f, turn, &request);
    let call = tokio::spawn(async move {
        state
            .authorize_account_change(&command, &current(), &request)
            .await
    });
    let card = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(card) = f
                .state
                .owned
                .session_store
                .get_session(&f.request.session_id)
                .unwrap()
                .active_interactions()
                .first()
            {
                break card.id().to_owned();
            }
            assert!(!call.is_finished(), "ended with no card: {:?}", call.is_finished());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the raise went to a card");
    f.state
        .resolve_terminal_runtime_interaction(
            &f.request.session_id,
            &card,
            "deny",
            None,
            Some("local"),
        )
        .await
        .unwrap();
    let error = call.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("denied"), "{error}");
    card
}

#[tokio::test]
async fn a_raise_beyond_the_grant_or_outside_it_goes_to_a_card() {
    let f = fixture();
    let turn = granted(&f, machines(4));
    // Five machines, the spend cap, and both at once are not what "4 machines" names.
    asks_a_card(&f, &turn, set(5, 10_000_000)).await;
    asks_a_card(&f, &turn, set(2, 15_000_000)).await;
    asks_a_card(&f, &turn, set(4, 15_000_000)).await;
}

#[tokio::test]
async fn a_window_that_names_nothing_covers_nothing() {
    let f = fixture();
    let turn = granted(&f, None);
    asks_a_card(&f, &turn, set(3, 10_000_000)).await;
}

#[tokio::test]
async fn an_ended_window_covers_nothing() {
    let f = fixture();
    let turn = granted(&f, machines(4));
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .remove(&turn.entry_id);
    asks_a_card(&f, &turn, set(3, 10_000_000)).await;
}

#[tokio::test]
async fn a_lower_needs_no_say_so() {
    let f = fixture();
    let turn = granted(&f, None);
    let request = set(1, 5_000_000);
    let approval = f
        .state
        .authorize_account_change(&from_the_agent(&f, &turn, &request), &current(), &request)
        .await
        .unwrap();
    assert!(approval.is_none());
    no_card(&f);
}

#[tokio::test]
async fn the_person_changes_their_own_limits_without_a_card() {
    let f = fixture();
    let request = set(9, 99_000_000);
    let command = KernelCommand::from_local_request(
        "person",
        None,
        None,
        &LocalDaemonRequest::ControlPlane(request.clone()),
    );
    let approval = f
        .state
        .authorize_account_change(&command, &current(), &request)
        .await
        .unwrap();
    assert!(approval.is_none());
    no_card(&f);
}

#[tokio::test]
async fn a_raise_after_the_limits_moved_is_refused_before_any_card() {
    let f = fixture();
    let turn = granted(&f, machines(4));
    let request = ControlPlaneRequest::SetLimits {
        expected_version: 0,
        machines_at_once: 3,
        spend_per_day_micros: 10_000_000,
        approval: None,
    };
    let error = f
        .state
        .authorize_account_change(&from_the_agent(&f, &turn, &request), &current(), &request)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("read them again"), "{error}");
    no_card(&f);
}

#[tokio::test]
async fn the_owners_own_sudo_prompt_names_the_grant() {
    let mut f = fixture();
    f.request.prompt = "/sudo raise limits to 4 machines".into();
    let window = open(&f, None).await;
    assert_eq!(window.limit_grant, machines(4));
    let live = f
        .state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .get(&window.entry_id)
        .unwrap()
        .limit_grant
        .clone();
    assert_eq!(live, machines(4), "the live window keeps the parsed grant");
}
