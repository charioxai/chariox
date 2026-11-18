//! The person's Inbox on this kernel (docs/NOTIFICATION_CENTER_PLAN.md section 4): every card
//! for the owner, from every Room they own here. A needs-you card is the Room's pending
//! interaction itself, so it closes everywhere when it is answered anywhere; a notice is what an
//! agent sent with `chariox.notify_person`. A workflow card is a run of a workflow the person
//! answers for that failed or holds, and a flag is a subscription that stopped delivering; both
//! are read live and leave when resolved. Read, snooze and mute are kept by the Room owner's
//! store so they follow the person to every client.
use crate::durable_state::workflow_notifications::naming::workflow_name;
use crate::durable_state::{PersonInboxState, StoredNotice};
use crate::error::DaemonError;
use crate::local::{
    InboxCardChange, LocalDaemonResponse, NotificationSubscriptionHealth,
    NotificationSubscriptionRow, PersonInboxCard, PersonInboxCardBody, PersonInboxFlag,
    PersonInboxNotice, PersonInboxPriority, PersonInboxWorkflowRun, SetInboxCardStateRequest,
    SetInboxMuteRequest,
};
use crate::runtime::state::KernelRuntimeState;
use crate::session::{
    unix_epoch_ms, RuntimeSession, WorkflowDefinition, WorkflowRun, WorkflowRunOutcome,
};
use std::collections::{BTreeMap, BTreeSet};

/// What an agent sends with `chariox.notify_person`.
pub(crate) struct PersonNotice {
    pub(crate) title: String,
    pub(crate) body: Option<String>,
    pub(crate) link: Option<String>,
    pub(crate) priority: PersonInboxPriority,
}

pub(crate) fn execute_person_inbox_request(
    runtime_state: &KernelRuntimeState,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let inbox = OwnedInbox::read(runtime_state, caller_user_id, unix_epoch_ms())?;
    Ok(inbox.response())
}

pub(crate) fn execute_set_inbox_card_state(
    runtime_state: &KernelRuntimeState,
    request: SetInboxCardStateRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let now = unix_epoch_ms();
    let inbox = OwnedInbox::read(runtime_state, caller_user_id, now)?;
    if let InboxCardChange::Snooze {
        until_ms: Some(until),
    } = request.change
    {
        if until <= now {
            return Err(refused("A snooze ends in the future. Pick a later time."));
        }
    }
    let mut by_owner = BTreeMap::<String, Vec<String>>::new();
    for card_id in request.card_ids {
        let owner = match (inbox.card_owner(&card_id), &request.change) {
            (Some((owner, CardKind::Notice)), _) => owner,
            (Some((owner, CardKind::NeedsYou)), InboxCardChange::Snooze { .. }) => owner,
            (
                Some((owner, CardKind::Workflow | CardKind::Flag)),
                InboxCardChange::Snooze { .. },
            ) => owner,
            (Some((_, CardKind::NeedsYou)), InboxCardChange::Read { .. }) => {
                return Err(refused(
                    "A needs-you card is not marked read: it leaves when it is answered.",
                ))
            }
            (Some((_, CardKind::Workflow)), InboxCardChange::Read { .. }) => {
                return Err(refused(
                    "A workflow card is not marked read: it leaves when you open the run.",
                ))
            }
            (Some((_, CardKind::Flag)), InboxCardChange::Read { .. }) => return Err(refused(
                "A flag is not marked read: it leaves when the subscription is fixed or removed.",
            )),
            (None, _) => {
                return Err(refused(format!(
                    "No card {card_id} in your Inbox. Read the Inbox again and pick a card from it."
                )))
            }
        };
        by_owner.entry(owner).or_default().push(card_id);
    }
    let store = runtime_state.durable_state_store();
    for (owner, card_ids) in by_owner {
        store.set_person_inbox_card_state(&owner, card_ids, request.change.clone(), now)?;
    }
    runtime_state.record_person_inbox_change();
    execute_person_inbox_request(runtime_state, caller_user_id)
}

/// A mute quiets one agent's notices: they arrive read. Its needs-you cards still show, because
/// the agent waits for an answer.
pub(crate) fn execute_set_inbox_mute(
    runtime_state: &KernelRuntimeState,
    request: SetInboxMuteRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let session = runtime_state
        .list_session_health_snapshots()
        .into_iter()
        .find(|session| {
            session.id() == request.origin.session_id
                && runtime_state.acts_for_room_owner(session, caller_user_id)
        })
        .ok_or_else(|| {
            refused(format!(
                "No Room {} of yours on this kernel. Mute an agent of a Room you own.",
                request.origin.session_id
            ))
        })?;
    runtime_state.durable_state_store().set_person_inbox_mute(
        session.owner_user_id(),
        request.origin,
        request.muted,
    )?;
    runtime_state.record_person_inbox_change();
    execute_person_inbox_request(runtime_state, caller_user_id)
}

/// Stores a notice from an agent of `session` for the Room's owner; a guest's agent has no
/// person to notify here, because a card in someone else's Room stays in that Room.
pub(crate) fn notify_person(
    runtime_state: &KernelRuntimeState,
    session: &RuntimeSession,
    agent_id: &str,
    agent_owner_user_id: &str,
    notice: PersonNotice,
) -> Result<String, DaemonError> {
    if !runtime_state.acts_for_room_owner(session, agent_owner_user_id) {
        return Err(refused(
            "chariox.notify_person notifies the Room's owner; a guest's agent cannot use it. Ask in the Room instead.",
        ));
    }
    let title = notice.title.trim();
    if title.is_empty() || title.contains('\n') {
        return Err(refused(
            "The title is one line of text. Put details in body.",
        ));
    }
    let created_at_ms = unix_epoch_ms();
    let stored = runtime_state
        .durable_state_store()
        .record_person_notice(StoredNotice {
            card_id: format!("notice_{created_at_ms}_{:016x}", rand::random::<u64>()),
            owner_user_id: session.owner_user_id().to_owned(),
            session_id: session.id().to_owned(),
            agent_id: agent_id.to_owned(),
            title: title.to_owned(),
            body: notice.body.filter(|body| !body.trim().is_empty()),
            link: notice.link.filter(|link| !link.trim().is_empty()),
            priority: notice.priority,
            created_at_ms,
            read: false,
        })?;
    runtime_state.record_person_inbox_change();
    Ok(stored.card_id)
}

enum CardKind {
    NeedsYou,
    Notice,
    Workflow,
    Flag,
}

/// The caller's Rooms on this kernel, the stored Inbox state of their owners and the
/// subscriptions of those Rooms that stopped delivering.
struct OwnedInbox {
    sessions: Vec<RuntimeSession>,
    state: PersonInboxState,
    flags: Vec<NotificationSubscriptionRow>,
}

impl OwnedInbox {
    fn read(
        runtime_state: &KernelRuntimeState,
        caller_user_id: &str,
        now_ms: u64,
    ) -> Result<Self, DaemonError> {
        let sessions = runtime_state
            .list_session_health_snapshots()
            .into_iter()
            .filter(|session| runtime_state.acts_for_room_owner(session, caller_user_id))
            .collect::<Vec<_>>();
        let session_ids = sessions
            .iter()
            .map(|session| session.id())
            .collect::<BTreeSet<_>>();
        let owners = sessions
            .iter()
            .map(|session| session.owner_user_id())
            .collect::<BTreeSet<_>>();
        let mut state = PersonInboxState::default();
        for owner in owners {
            let owned = runtime_state
                .durable_state_store()
                .person_inbox_state(owner, now_ms)?;
            state.notices.extend(
                owned
                    .notices
                    .into_iter()
                    .filter(|notice| session_ids.contains(notice.session_id.as_str())),
            );
            state.snoozes.extend(owned.snoozes);
            state.mutes.extend(
                owned
                    .mutes
                    .into_iter()
                    .filter(|origin| session_ids.contains(origin.session_id.as_str())),
            );
        }
        state.mutes.sort();
        let flags = runtime_state
            .notification_subscription_rows(caller_user_id)?
            .into_iter()
            .filter(|row| flagged(row.health))
            .collect();
        Ok(Self {
            sessions,
            state,
            flags,
        })
    }

    fn card_owner(&self, card_id: &str) -> Option<(String, CardKind)> {
        if let Some(notice) = self
            .state
            .notices
            .iter()
            .find(|notice| notice.card_id == card_id)
        {
            return Some((notice.owner_user_id.clone(), CardKind::Notice));
        }
        if let Some(flag) = self.flags.iter().find(|row| flag_card_id(row) == card_id) {
            return Some((flag.subscription.owner_user_id.clone(), CardKind::Flag));
        }
        if let Some(session) = self
            .sessions
            .iter()
            .find(|session| person_runs(session).any(|(run, _, _)| run_card_id(run) == card_id))
        {
            return Some((session.owner_user_id().to_owned(), CardKind::Workflow));
        }
        self.sessions
            .iter()
            .find(|session| {
                session
                    .named_active_interactions()
                    .iter()
                    .any(|interaction| interaction.id() == card_id)
            })
            .map(|session| (session.owner_user_id().to_owned(), CardKind::NeedsYou))
    }

    /// High-priority notices first, then newest first.
    fn response(self) -> LocalDaemonResponse {
        let snoozed = |card_id: &str| self.state.snoozes.get(card_id).copied();
        let mut cards = self
            .sessions
            .iter()
            .flat_map(|session| {
                session
                    .named_active_interactions()
                    .into_iter()
                    .map(|interaction| PersonInboxCard {
                        card_id: interaction.id().to_owned(),
                        session_id: session.id().to_owned(),
                        snoozed_until_ms: snoozed(interaction.id()),
                        body: PersonInboxCardBody::NeedsYou {
                            interaction: Box::new(interaction),
                        },
                    })
            })
            .collect::<Vec<_>>();
        cards.extend(self.state.notices.iter().map(|notice| {
            let agent_name = self
                .sessions
                .iter()
                .find(|session| session.id() == notice.session_id)
                .and_then(|session| session.agent_display_name(&notice.agent_id));
            PersonInboxCard {
                card_id: notice.card_id.clone(),
                session_id: notice.session_id.clone(),
                snoozed_until_ms: snoozed(&notice.card_id),
                body: PersonInboxCardBody::Notice(PersonInboxNotice {
                    agent_id: notice.agent_id.clone(),
                    agent_name,
                    title: notice.title.clone(),
                    body: notice.body.clone(),
                    link: notice.link.clone(),
                    priority: notice.priority,
                    created_at_ms: notice.created_at_ms,
                    read: notice.read,
                }),
            }
        }));
        cards.extend(self.sessions.iter().flat_map(|session| {
            person_runs(session).map(|(run, workflow, outcome)| PersonInboxCard {
                card_id: run_card_id(run),
                session_id: session.id().to_owned(),
                snoozed_until_ms: snoozed(&run_card_id(run)),
                body: PersonInboxCardBody::Workflow(PersonInboxWorkflowRun {
                    workflow_id: run.workflow_id().to_owned(),
                    workflow_name: workflow_name(workflow),
                    run_id: run.id().to_owned(),
                    run_number: run.number(),
                    outcome,
                    at_ms: run.completed_at_ms().unwrap_or(run.created_at_ms()),
                }),
            })
        }));
        cards.extend(self.flags.iter().map(|row| PersonInboxCard {
            card_id: flag_card_id(row),
            session_id: row.subscription.session_id.clone(),
            snoozed_until_ms: snoozed(&flag_card_id(row)),
            body: PersonInboxCardBody::Flag(PersonInboxFlag {
                subscription_id: row.subscription.subscription_id.clone(),
                subscriber_name: row.subscriber_name.clone(),
                source_name: row.source.as_ref().map(|source| source.name.clone()),
                health: row.health,
                at_ms: row.loop_stopped_at_ms.or(row.last_fired_at_ms),
            }),
        }));
        cards.sort_by(|left, right| {
            sort_key(right)
                .cmp(&sort_key(left))
                .then_with(|| left.card_id.cmp(&right.card_id))
        });
        LocalDaemonResponse::PersonInbox {
            cards,
            mutes: self.state.mutes,
        }
    }
}

fn sort_key(card: &PersonInboxCard) -> (bool, u64) {
    match &card.body {
        PersonInboxCardBody::NeedsYou { interaction } => (false, interaction.requested_at_ms()),
        PersonInboxCardBody::Notice(notice) => (
            notice.priority == PersonInboxPriority::High,
            notice.created_at_ms,
        ),
        PersonInboxCardBody::Workflow(run) => (false, run.at_ms),
        PersonInboxCardBody::Flag(flag) => (false, flag.at_ms.unwrap_or_default()),
    }
}

/// Runs of the Room's workflows that the person answers for and that wait on someone.
fn person_runs(
    session: &RuntimeSession,
) -> impl Iterator<Item = (&WorkflowRun, &WorkflowDefinition, WorkflowRunOutcome)> {
    session.workflow_runs().iter().filter_map(move |run| {
        let workflow = session.workflow(run.workflow_id())?;
        let outcome = run.outcome()?;
        (run.attention() && workflow.responsible().is_person()).then_some((run, workflow, outcome))
    })
}

fn run_card_id(run: &WorkflowRun) -> String {
    format!("run:{}", run.id())
}

fn flag_card_id(row: &NotificationSubscriptionRow) -> String {
    format!("flag:{}", row.subscription.subscription_id)
}

/// A subscription that cannot deliver until the person acts; a pause is the person's own doing.
fn flagged(health: NotificationSubscriptionHealth) -> bool {
    !matches!(
        health,
        NotificationSubscriptionHealth::Ok | NotificationSubscriptionHealth::Paused
    )
}

fn refused(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "person_inbox",
        message: message.into(),
    }
}
