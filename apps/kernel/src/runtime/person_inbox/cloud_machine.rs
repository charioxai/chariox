//! Machine events the connected service reports (a failed copy, a hard stop, a failed rental)
//! become Inbox notices written by the kernel, so every client and every agent reads them without
//! a web tab open. The service sends a closed reason code; the sentence is written here.
use super::*;
use crate::durable_state::{CLOUD_MACHINE_NOTICE_PREFIX, RETENTION_MS};
use crate::local::{ControlPlaneMachineRow, MachineEventReason};

/// States a machine can leave by itself, so the connected service may still report an event.
const CHANGING_STATES: [&str; 7] = [
    "requested",
    "provisioning",
    "bootstrapping",
    "awaiting_context",
    "ready",
    "starting",
    "stopping",
];

/// What one read of the owner's machines changes in the Inbox.
#[derive(Debug, PartialEq)]
pub(crate) struct CloudMachinePlan {
    pub(crate) raise: Vec<StoredNotice>,
    pub(crate) clear: Vec<String>,
}

/// One card per machine, reason and moment the machine entered that state: a restart finds the
/// card it raised before, a new failure of the same machine is a new card.
fn card_id(row: &ControlPlaneMachineRow, reason: MachineEventReason, since_ms: u64) -> String {
    let code = match reason {
        MachineEventReason::CopyFailed => "copy_failed",
        MachineEventReason::HardStop => "hard_stop",
        MachineEventReason::RentalFailed => "rental_failed",
    };
    format!(
        "{CLOUD_MACHINE_NOTICE_PREFIX}{}:{code}:{since_ms}",
        row.allocation_ref
    )
}

fn since_ms(row: &ControlPlaneMachineRow) -> Option<u64> {
    let at = chrono::DateTime::parse_from_rfc3339(&row.since).ok()?;
    u64::try_from(at.timestamp_millis()).ok()
}

/// A machine name is one line of what the person typed or the service chose.
fn one_line(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

fn sentence(reason: MachineEventReason, name: &str) -> (String, String) {
    let name = one_line(name);
    match reason {
        MachineEventReason::CopyFailed => (
            format!("Copying to {name} failed"),
            format!("{name} is stopped. Open Machines to retry the copy, start fresh or delete it."),
        ),
        MachineEventReason::HardStop => (
            format!("{name} was stopped at its time limit"),
            format!("Chariox stopped {name} at the limit set when it was rented. Open Machines to start it again."),
        ),
        MachineEventReason::RentalFailed => (
            format!("{name} could not be started"),
            format!("The rental of {name} failed. Open Machines to try again or delete it."),
        ),
    }
}

/// Rows whose event still stands: a reason inside the notice retention.
fn standing(
    machines: &[ControlPlaneMachineRow],
    now_ms: u64,
) -> impl Iterator<Item = (&ControlPlaneMachineRow, MachineEventReason, u64)> {
    machines.iter().filter_map(move |row| {
        let reason = row.reason?;
        let since = since_ms(row)?;
        (now_ms.saturating_sub(since) <= RETENTION_MS).then_some((row, reason, since))
    })
}

/// What to raise and clear for `machines`, given the cloud-machine cards already in the Inbox.
/// A card that stays in the Inbox is never raised again, whether it was read or not.
pub(crate) fn plan(
    owner_user_id: &str,
    machines: &[ControlPlaneMachineRow],
    existing_card_ids: &BTreeSet<String>,
    now_ms: u64,
) -> CloudMachinePlan {
    let mut wanted = BTreeSet::new();
    let mut raise = Vec::new();
    for (row, reason, since) in standing(machines, now_ms) {
        let id = card_id(row, reason, since);
        wanted.insert(id.clone());
        if existing_card_ids.contains(&id) {
            continue;
        }
        let (title, body) = sentence(reason, &row.name);
        raise.push(StoredNotice {
            card_id: id,
            owner_user_id: owner_user_id.to_owned(),
            session_id: String::new(),
            agent_id: format!("{CLOUD_MACHINE_NOTICE_PREFIX}{}", row.allocation_ref),
            title,
            body: Some(body),
            link: None,
            priority: PersonInboxPriority::High,
            created_at_ms: since,
            read: false,
            folded: 0,
        });
    }
    let clear = existing_card_ids
        .iter()
        .filter(|id| !wanted.contains(*id))
        .cloned()
        .collect();
    CloudMachinePlan { raise, clear }
}

/// Whether the connected service can still report an event for these machines without the
/// kernel being asked: a machine that can change by itself, or an event whose card can still clear.
pub(crate) fn needs_watching(machines: &[ControlPlaneMachineRow], now_ms: u64) -> bool {
    standing(machines, now_ms).next().is_some()
        || machines
            .iter()
            .any(|row| CHANGING_STATES.contains(&row.state.as_str()))
}

impl KernelRuntimeState {
    /// Raise and clear the owner's cloud-machine cards for one read of their machines.
    pub(crate) fn reconcile_cloud_machine_notices(
        &self,
        machines: &[ControlPlaneMachineRow],
    ) -> Result<(), DaemonError> {
        let owner = self.provider_account_authority_owner_user_id(&self.kernel_owner_user_id());
        let now = unix_epoch_ms();
        let store = self.durable_state_store();
        let existing = store
            .person_inbox_state(&owner, now)?
            .notices
            .into_iter()
            .filter(StoredNotice::is_cloud_machine)
            .map(|notice| notice.card_id)
            .collect::<BTreeSet<_>>();
        let plan = plan(&owner, machines, &existing, now);
        if plan.raise.is_empty() && plan.clear.is_empty() {
            return Ok(());
        }
        for card_id in &plan.clear {
            store.remove_person_notice(&owner, card_id)?;
        }
        for notice in plan.raise {
            store.replace_person_notice(notice)?;
        }
        self.record_person_inbox_change();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn row(allocation: &str, state: &str, reason: Option<MachineEventReason>, age_ms: u64) -> ControlPlaneMachineRow {
        let since = chrono::DateTime::from_timestamp_millis((NOW - age_ms) as i64).unwrap();
        ControlPlaneMachineRow {
            allocation_ref: allocation.into(),
            name: "Build box".into(),
            state: state.into(),
            revision: 3,
            since: since.to_rfc3339(),
            kernel_id: None,
            machine_id: None,
            available_actions: vec![],
            reason,
        }
    }

    #[test]
    fn each_reason_raises_one_high_card_in_the_kernel_s_words() {
        for (reason, title) in [
            (MachineEventReason::CopyFailed, "Copying to Build box failed"),
            (MachineEventReason::HardStop, "Build box was stopped at its time limit"),
            (MachineEventReason::RentalFailed, "Build box could not be started"),
        ] {
            let plan = plan("owner", &[row("m1", "failed", Some(reason), 1_000)], &BTreeSet::new(), NOW);
            assert_eq!(plan.raise.len(), 1);
            let notice = &plan.raise[0];
            assert_eq!(notice.title, title);
            assert_eq!(notice.priority, PersonInboxPriority::High);
            assert!(notice.is_cloud_machine());
            assert!(plan.clear.is_empty());
        }
    }

    #[test]
    fn a_machine_the_owner_stopped_raises_nothing() {
        let plan = plan("owner", &[row("m1", "stopped", None, 1_000)], &BTreeSet::new(), NOW);
        assert_eq!(plan, CloudMachinePlan { raise: vec![], clear: vec![] });
    }

    #[test]
    fn a_card_already_in_the_inbox_is_not_raised_again() {
        let machines = [row("m1", "failed", Some(MachineEventReason::CopyFailed), 1_000)];
        let first = plan("owner", &machines, &BTreeSet::new(), NOW);
        let existing = first.raise.iter().map(|n| n.card_id.clone()).collect();
        let again = plan("owner", &machines, &existing, NOW + 60_000);
        assert_eq!(again, CloudMachinePlan { raise: vec![], clear: vec![] });
    }

    #[test]
    fn an_event_older_than_the_retention_raises_nothing() {
        let old = row("m1", "failed", Some(MachineEventReason::CopyFailed), RETENTION_MS + 1);
        assert!(plan("owner", &[old], &BTreeSet::new(), NOW).raise.is_empty());
    }

    #[test]
    fn the_card_clears_when_the_machine_leaves_that_state_and_a_new_failure_is_a_new_card() {
        let failed = row("m1", "failed", Some(MachineEventReason::CopyFailed), 5_000);
        let first = plan("owner", &[failed], &BTreeSet::new(), NOW);
        let existing: BTreeSet<_> = first.raise.iter().map(|n| n.card_id.clone()).collect();
        let started = row("m1", "ready", None, 1_000);
        let cleared = plan("owner", &[started], &existing, NOW);
        assert_eq!(cleared.clear, existing.iter().cloned().collect::<Vec<_>>());
        let failed_again = row("m1", "failed", Some(MachineEventReason::CopyFailed), 100);
        let second = plan("owner", &[failed_again], &existing, NOW);
        assert_eq!(second.raise.len(), 1);
        assert_ne!(second.raise[0].card_id, first.raise[0].card_id);
        assert_eq!(second.clear.len(), 1, "the earlier failure's card leaves");
    }

    #[test]
    fn a_machine_name_cannot_break_the_title_line() {
        let mut machine = row("m1", "failed", Some(MachineEventReason::CopyFailed), 1_000);
        machine.name = "Box\nwith a break".into();
        let plan = plan("owner", &[machine], &BTreeSet::new(), NOW);
        assert!(!plan.raise[0].title.contains('\n'));
    }

    #[test]
    fn only_machines_that_can_still_report_an_event_are_watched() {
        assert!(!needs_watching(&[], NOW));
        assert!(!needs_watching(&[row("m1", "stopped", None, 1_000)], NOW));
        assert!(!needs_watching(&[row("m1", "deleted", None, 1_000)], NOW));
        for state in ["provisioning", "awaiting_context", "ready", "stopping"] {
            assert!(needs_watching(&[row("m1", state, None, 1_000)], NOW), "{state}");
        }
        assert!(needs_watching(
            &[row("m1", "failed", Some(MachineEventReason::RentalFailed), 1_000)],
            NOW
        ));
        assert!(!needs_watching(
            &[row("m1", "failed", Some(MachineEventReason::RentalFailed), RETENTION_MS + 1)],
            NOW
        ));
    }
}
