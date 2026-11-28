//! Row 569: what lets an agent raise the account's machine limits: the owner's own `/sudo` naming
//! the raise in its exact scope. Missing authority is refused; an agent's words decide nothing.
use super::limit_grant::LimitGrant;
use super::tests::{fixture, running, Fixture};
use super::window_tests::open;
use super::*;
use crate::local::{
    AccountLimitValues, ControlPlaneRequest, KernelConnectionClass, LimitApprovalKind,
    LocalDaemonRequest,
};
use crate::runtime::command::KernelCommand;
use crate::runtime::state::machine_acquisition::limits::AccountLimits;

fn current() -> AccountLimits {
    AccountLimits {
        values: AccountLimitValues {
            machines_at_once: 2,
            spend_per_day_micros: 10_000_000,
        },
        currency: "EUR".into(),
        version: 1,
        machines_in_use: None,
        spend_today_micros: None,
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

async fn refuses_without_named_authority(f: &Fixture, turn: &KernelSudoTurn, request: ControlPlaneRequest) -> String {
    let error=f.state.authorize_account_change(&from_the_agent(f,turn,&request),&current(),&request)
        .await.expect_err("the named owner window must cover every requested raise");
    assert!(matches!(&error,DaemonError::MachineRequest {code,..} if code=="owner_action_required"),"{error}");
    let message=error.to_string(); assert!(message.contains("/sudo"),"{message}"); no_card(f); message
}

#[tokio::test]
async fn a_raise_beyond_or_outside_the_named_grant_is_refused_without_a_card() {
    let f = fixture();
    let turn = granted(&f, machines(4));
    // Five machines, the spend cap, and both at once are not what "4 machines" names.
    refuses_without_named_authority(&f, &turn, set(5, 10_000_000)).await;
    refuses_without_named_authority(&f, &turn, set(2, 15_000_000)).await;
    refuses_without_named_authority(&f, &turn, set(4, 15_000_000)).await;
}

#[tokio::test]
async fn a_window_that_names_nothing_covers_nothing() {
    let f = fixture();
    let turn = granted(&f, None);
    refuses_without_named_authority(&f, &turn, set(3, 10_000_000)).await;
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
    refuses_without_named_authority(&f, &turn, set(3, 10_000_000)).await;
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


#[tokio::test]
async fn a_no_ask_grant_never_authorizes_the_account_scope() {
    let f=fixture();
    let turn=granted(&f,LimitGrant::parse("raise no-ask limits to 4 machines"));
    refuses_without_named_authority(&f,&turn,set(3,10_000_000)).await;
    no_card(&f);
}

#[tokio::test]
async fn runtime_authority_keeps_the_named_currency_and_exact_window_binding() {
    let f=fixture();
    let turn=granted(&f,LimitGrant::parse("raise account limits to 15 EUR a day"));
    let request=set(2,15_000_000); let mut other_currency=current(); other_currency.currency="USD".into();
    let error=f.state.authorize_account_change(&from_the_agent(&f,&turn,&request),&other_currency,&request).await.unwrap_err();
    assert!(matches!(error,DaemonError::MachineRequest {ref code,..} if code=="owner_action_required")); no_card(&f);
    f.state.owned.sudo_turns.lock().unwrap().get_mut(&turn.entry_id).unwrap().agent_id="another-agent".into();
    refuses_without_named_authority(&f,&turn,request).await; no_card(&f);
}
