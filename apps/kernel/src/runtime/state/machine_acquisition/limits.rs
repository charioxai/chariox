//! Row 569: the account's machine limits and the owner's limits for acquiring without asking, as
//! agents read and change them. The person decides every raise; an agent lowers directly.
//!
//! Authority is the kernel's, not the page's: whatever path an agent takes (the tools, the CLI, a
//! raw control-plane request), `authorize_account_change` decides before the connected service is
//! asked, and only it attaches the approval the service checks.
use super::*;
use crate::local::{
    AccountLimitValues, AccountLimitsReading, LimitApproval, LimitApprovalKind, LimitScope,
    MachineLimitsReading, NoAskLimitsReading,
};
use crate::local::MachineAllocationCapability;
use crate::runtime::state::RequestedRaise;

/// A raise card's id starts with this. It is no machine acquisition decision, so no agent answers it.
pub(crate) const LIMITS_CARD_PREFIX: &str = "limits-raise-";

/// Whether `id` is a card raising limits: the person's alone, never answered for them by an agent.
pub(crate) fn is_limits_raise_decision(id: &str) -> bool {
    id.starts_with(LIMITS_CARD_PREFIX)
}

/// The account's limits as the connected service answered them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccountLimits {
    pub(crate) values: AccountLimitValues,
    pub(crate) currency: String,
    pub(crate) version: u64,
    pub(crate) machines_in_use: Option<u32>,
    pub(crate) spend_today_micros: Option<u64>,
}

impl AccountLimits {
    pub(crate) fn from_response(response: &crate::local::ControlPlaneResponse) -> Option<Self> {
        match response {
            crate::local::ControlPlaneResponse::Limits {
                machines_at_once,
                spend_per_day_micros,
                currency,
                version,
                machines_in_use,
                spend_today_micros,
            } => Some(Self {
                values: AccountLimitValues {
                    machines_at_once: *machines_at_once,
                    spend_per_day_micros: *spend_per_day_micros,
                },
                currency: currency.clone(),
                version: *version,
                machines_in_use: *machines_in_use,
                spend_today_micros: *spend_today_micros,
            }),
            _ => None,
        }
    }

    /// The same limits at the same version and currency: what an approval was given for. How many machines run, or what is
    /// spent today, moves while the person decides and is not what they approved.
    pub(crate) fn same_limits(&self, other: &Self) -> bool {
        self.values == other.values && self.currency == other.currency && self.version == other.version
    }

    fn reading(&self) -> AccountLimitsReading {
        AccountLimitsReading {
            machines_at_once: self.values.machines_at_once,
            machines_in_use: self.machines_in_use,
            spend_today_micros: self.spend_today_micros,
            spend_per_day_micros: self.values.spend_per_day_micros,
            currency: self.currency.clone(),
            version: self.version,
        }
    }
}

/// What the kernel knows of the account's limits when it admits an acquisition without asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AccountBound {
    /// The connected service has no account limits (or the owner allows nothing without asking).
    None,
    /// The service has them and the read failed: nothing goes ahead without asking on a guess.
    Unread,
    Limits(AccountLimits),
}

pub(super) fn plural_machines(count: u32) -> String {
    plural(count)
}

fn plural(count: u32) -> String {
    format!("{count} machine{}", if count == 1 { "" } else { "s" })
}

pub(super) fn amount_text(micros: u64, currency: &str) -> String {
    if micros % 10_000 == 0 {
        format!("{:.2} {currency}", micros as f64 / 1_000_000.0)
    } else {
        format!("{} {currency}", money(micros))
    }
}

/// "the account allows 3 machines": the neutral words for a machine count the account bounds.
pub(super) fn account_allows_machines(count: u32) -> String {
    format!("the account allows {}", plural(count))
}

/// "the account allows 10.00 EUR a day".
pub(super) fn account_allows_spend(micros: u64, currency: &str) -> String {
    format!("the account allows {} a day", amount_text(micros, currency))
}

/// The owner's no-ask limits and what applies once the account's bound them: the smaller of each pair.
pub(crate) fn no_ask_reading(
    settings: &crate::config::UserAgentsConfig,
    account: Option<&AccountLimits>,
) -> NoAskLimitsReading {
    let machines = settings
        .offload_max_machines
        .unwrap_or(crate::config::DEFAULT_OFFLOAD_MAX_MACHINES);
    let cap = settings.offload_daily_cap_micros;
    let effective_machines = account.map_or(machines, |a| machines.min(a.values.machines_at_once));
    let effective_cap = match (cap, account) {
        (cap, None) => cap,
        (None, Some(a)) => Some(a.values.spend_per_day_micros),
        (Some(cap), Some(a)) => Some(cap.min(a.values.spend_per_day_micros)),
    };
    NoAskLimitsReading {
        enabled: settings.offload_without_asking == Some(true),
        machines_at_once: machines,
        spend_per_day_micros: cap,
        classes: settings.offload_classes.clone(),
        effective_machines_at_once: effective_machines,
        effective_spend_per_day_micros: effective_cap,
        clamped: effective_machines < machines || effective_cap != cap,
    }
}

/// Which fields a change would raise above their current value.
fn raised(from: &AccountLimitValues, to: &AccountLimitValues) -> RequestedRaise {
    RequestedRaise {
        machines_at_once: (to.machines_at_once > from.machines_at_once)
            .then_some(to.machines_at_once),
        spend_per_day_micros: (to.spend_per_day_micros > from.spend_per_day_micros)
            .then_some(to.spend_per_day_micros),
    }
}

fn change_text(
    scope: LimitScope,
    from: &AccountLimitValues,
    to: &AccountLimitValues,
    currency: &str,
) -> String {
    let (machines, spend) = match scope {
        LimitScope::Account => ("Machines at once", "Spend per day"),
        LimitScope::NoAsk => (
            "Machines at once without asking",
            "Spend per day without asking",
        ),
    };
    let mut parts = Vec::new();
    if from.machines_at_once != to.machines_at_once {
        parts.push(format!(
            "{machines}: from {} to {}.",
            from.machines_at_once, to.machines_at_once
        ));
    }
    if from.spend_per_day_micros != to.spend_per_day_micros {
        parts.push(format!(
            "{spend}: from {} to {}.",
            amount_text(from.spend_per_day_micros, currency),
            amount_text(to.spend_per_day_micros, currency)
        ));
    }
    parts.join(" ")
}

impl KernelRuntimeState {
    /// The account limits an agent's change may apply, or the approval the service needs for it.
    /// `None` approval: the person changes their own limits, or the agent only lowers them.
    pub(crate) async fn authorize_account_change(
        &self,
        command: &KernelCommand,
        current: &AccountLimits,
        request: &ControlPlaneRequest,
    ) -> Result<Option<LimitApproval>, DaemonError> {
        let ControlPlaneRequest::SetLimits {
            expected_version,
            machines_at_once,
            spend_per_day_micros,
            approval,
        } = request
        else {
            return Ok(None);
        };
        let Some(caller) = self.agent_caller(command)? else {
            return Ok(None);
        };
        if approval.is_some() {
            return Err(acquisition_error(
                "invalid_request",
                "The person's approval of a raise is attached by the kernel, never sent by an agent",
            ));
        }
        if *expected_version != current.version {
            return Err(stale_limits());
        }
        let to = AccountLimitValues {
            machines_at_once: *machines_at_once,
            spend_per_day_micros: *spend_per_day_micros,
        };
        if raised(&current.values, &to) == RequestedRaise::default() {
            return Ok(None);
        }
        let (kind, reference) = self
            .raise_authority(
                &caller,
                LimitScope::Account,
                &current.values,
                &to,
                &current.currency,
            )
            .await?;
        Ok(Some(LimitApproval {
            kind,
            reference,
            from: current.values,
            to,
        }))
    }

    /// The person's say-so for a raise: their `/sudo` naming it, else a card showing the exact values.
    async fn raise_authority(
        &self,
        caller: &AgentCaller,
        scope: LimitScope,
        from: &AccountLimitValues,
        to: &AccountLimitValues,
        currency: &str,
    ) -> Result<(LimitApprovalKind, String), DaemonError> {
        if let Some((entry, grant)) = self.sudo_limit_grant(caller) {
            if grant.covers(&raised(from, to), currency) {
                return Ok((LimitApprovalKind::Sudo, entry));
            }
        }
        let id = self.ask_limit_raise(caller, scope, from, to, currency).await?;
        Ok((LimitApprovalKind::Card, id))
    }

    /// What the owner's own `/sudo` named about limits, while that window is live and bound to this
    /// agent's running turn. A window that ended, or belongs to another agent, covers nothing.
    fn sudo_limit_grant(&self, caller: &AgentCaller) -> Option<(String, crate::runtime::state::LimitGrant)> {
        let agent = caller.agent.as_deref()?;
        let run = self
            .owned
            .provider_store
            .get_run_for_agent(&caller.room, agent)?;
        let turn = self.sudo_for_run(run.id()).ok()?;
        if turn.agent_id != agent || turn.session_id != caller.room {
            return None;
        }
        Some((turn.entry_id, turn.limit_grant?))
    }

    /// The person's card for a raise of the limits: shown to every client of the Room, answered only by the person,
    /// never by the requester or its sub-agents. Returns the card's id once it is approved.
    async fn ask_limit_raise(
        &self,
        caller: &AgentCaller,
        scope: LimitScope,
        from: &AccountLimitValues,
        to: &AccountLimitValues,
        currency: &str,
    ) -> Result<String, DaemonError> {
        let (suffix, title) = match scope {
            LimitScope::Account => ("asks to raise the account limits.", "account"),
            LimitScope::NoAsk => (
                "asks to raise your limits for machines it acquires without asking.",
                "no_ask",
            ),
        };
        self.ask_card(
            caller,
            &format!("{title}\0{from:?}\0{to:?}"),
            suffix,
            change_text(scope, from, to, currency),
        )
        .await
    }

    /// One card: who asks (`{name} {title_rest}`), what changes, and the id that follows from `seed`.
    async fn ask_card(
        &self,
        caller: &AgentCaller,
        seed: &str,
        title_rest: &str,
        description: String,
    ) -> Result<String, DaemonError> {
        let (room, owner, agent) = (&caller.room, &caller.owner, caller.agent.as_deref());
        // Every ask is its own card: one that was answered or withdrawn is never reopened by the same words.
        let id = format!(
            "{LIMITS_CARD_PREFIX}{:x}-{:016x}",
            sha2::Sha256::digest(
                format!("{room}\0{}\0{seed}", agent.unwrap_or_default()).as_bytes()
            ),
            rand::random::<u64>()
        );
        let name = agent
            .and_then(|agent| self.owned.agent_store.get_agent(agent).ok())
            .map(|known| known.alias().unwrap_or(known.agent_ref()).to_owned())
            .unwrap_or_else(|| "An agent".into());
        let title = format!("{name} {title_rest}");
        let mut interaction = RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            title,
            description,
            vec![
                RuntimeInteractionChoice::new(
                    "approve",
                    "Approve",
                    "approve",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
        );
        if let Some(agent) = agent {
            interaction = interaction.with_related_agent_id(agent.into());
        }
        if room.is_empty() {
            interaction = interaction.with_timeout_sec(ACQUISITION_DECISION_SECONDS);
        }
        let receiver = self
            .create_kernel_operation_interaction(
                crate::runtime::state::CardKind::OwnerControl,
                room,
                owner,
                interaction,
            )
            .await?;
        let mut pending = PendingAcquisition(Some((self.clone(), room.clone(), id.clone())));
        let asked = std::time::Instant::now();
        let waited =
            tokio::time::timeout(Duration::from_secs(ACQUISITION_DECISION_SECONDS), receiver).await;
        let why = match waited {
            Ok(Ok(answer)) if answer.status == "answered" => {
                pending.0 = None;
                if answer.choice_id.as_deref() == Some("approve") {
                    return Ok(id);
                }
                return Err(acquisition_error(
                    "limits_raise_denied",
                    "The person denied raising the limits, so nothing changed",
                ));
            }
            Ok(Ok(answer)) => match answer.reply {
                Some(reason) => format!("it could not be answered: {reason}"),
                None => format!("nobody answered it within {} s", asked.elapsed().as_secs()),
            },
            Ok(Err(_)) => format!(
                "the kernel withdrew it after {} s, before anyone answered",
                asked.elapsed().as_secs()
            ),
            Err(_) => format!("nobody answered it within {} s", asked.elapsed().as_secs()),
        };
        self.timeout_runtime_interaction(room, &id).await?;
        Err(acquisition_error(
            "limits_raise_unanswered",
            &format!("The person's card {id} closed without a decision: {why}. Nothing changed; ask again when the person can answer"),
        ))
    }

    /// Whether the owner lets agents acquire machines without asking.
    pub(crate) fn no_ask_enabled(&self) -> bool {
        self.owned
            .config_projection
            .snapshot()
            .user_config
            .agents
            .offload_without_asking
            == Some(true)
    }

    /// Reads the owner's no-ask limits and the account's, for the tool, the CLI and the Settings label.
    pub(crate) fn limits_reading(&self, account: Option<&AccountLimits>) -> MachineLimitsReading {
        let settings = self.owned.config_projection.snapshot().user_config.agents;
        MachineLimitsReading {
            account: account.map(AccountLimits::reading),
            no_ask: no_ask_reading(&settings, account),
        }
    }

    /// An agent's (or the owner's) change of the no-ask limits, as one transition: everything the person must say yes to is
    /// asked before anything is written, and the write happens only if the settings are still the ones the change was
    /// planned and approved against, so a denied card leaves nothing half done and an edit made meanwhile is never lost.
    /// A lower applies; a raise of an agent needs the person's `/sudo` or card. The owner's own change is the owner's.
    pub(super) async fn change_no_ask_limits(
        &self,
        caller: Option<AgentCaller>,
        machines_at_once: Option<u32>,
        spend_per_day_micros: Option<u64>,
        classes: Option<Vec<String>>,
        capability: &MachineAllocationCapability,
        account: Option<&AccountLimits>,
    ) -> Result<(), DaemonError> {
        let settings = self.owned.config_projection.snapshot().user_config.agents;
        let class_plan = classes
            .map(|classes| plan_no_ask_classes(&settings, classes, capability))
            .transpose()?;
        let from = AccountLimitValues {
            machines_at_once: settings
                .offload_max_machines
                .unwrap_or(crate::config::DEFAULT_OFFLOAD_MAX_MACHINES),
            // No cap of its own is the largest value: any number is a lower.
            spend_per_day_micros: settings.offload_daily_cap_micros.unwrap_or(u64::MAX),
        };
        let to = AccountLimitValues {
            machines_at_once: machines_at_once.unwrap_or(from.machines_at_once),
            spend_per_day_micros: spend_per_day_micros.unwrap_or(from.spend_per_day_micros),
        };
        if let Some(caller) = &caller {
            if let Some(plan) = class_plan.as_ref().filter(|plan| plan.adds) {
                self.ask_card(
                    caller,
                    &format!("classes\0{}\0{}", plan.current.join(","), plan.target.join(",")),
                    "asks to raise your limits for machines it acquires without asking.",
                    format!(
                        "Machine classes without asking: from {} to {}.",
                        plan.current.join(", "),
                        plan.target.join(", ")
                    ),
                )
                .await?;
            }
            if raised(&from, &to) != RequestedRaise::default() {
                let currency = account.map_or("", |a| a.currency.as_str());
                self.raise_authority(caller, LimitScope::NoAsk, &from, &to, currency)
                    .await?;
            }
        }
        let mut changes: Vec<(&'static str, Option<String>)> = Vec::new();
        if let Some(plan) = &class_plan {
            if plan.wanted.is_empty() {
                if settings.offload_classes.is_some() {
                    changes.push(("agents.offload_classes", None));
                }
            } else if settings.offload_classes.as_deref() != Some(plan.wanted.as_slice()) {
                changes.push(("agents.offload_classes", Some(plan.wanted.join(","))));
            }
        }
        if to.machines_at_once != from.machines_at_once {
            changes.push((
                "agents.offload_max_machines",
                Some(to.machines_at_once.to_string()),
            ));
        }
        if to.spend_per_day_micros != from.spend_per_day_micros {
            changes.push((
                "agents.offload_daily_cap_micros",
                Some(to.spend_per_day_micros.to_string()),
            ));
        }
        if changes.is_empty() {
            return Ok(());
        }
        self.write_no_ask_settings(settings, changes).await
    }

    /// Writes the planned changes in one step, only while the no-ask settings are still `expected`.
    async fn write_no_ask_settings(
        &self,
        expected: crate::config::UserAgentsConfig,
        changes: Vec<(&'static str, Option<String>)>,
    ) -> Result<(), DaemonError> {
        let config = self
            .with_authorized_app_side_effect(move |app| {
                let now = &app.config().user_config.agents;
                if now.offload_classes != expected.offload_classes
                    || now.offload_max_machines != expected.offload_max_machines
                    || now.offload_daily_cap_micros != expected.offload_daily_cap_micros
                {
                    return Err(stale_settings());
                }
                for (path, value) in changes {
                    match value {
                        Some(value) => {
                            app.set_user_config_value(path, value)?;
                        }
                        None => {
                            app.unset_user_config_value(path)?;
                        }
                    }
                }
                Ok(app.config().clone())
            })
            .await?;
        self.owned.config_projection.update(config);
        Ok(())
    }
}

/// The smallest class the service offers: the only one agents may acquire without asking when the person named none.
fn smallest_class(capability: &MachineAllocationCapability) -> Option<String> {
    capability
        .offers
        .iter()
        .min_by(|a, b| {
            (a.vcpu, a.ram_gb, a.rate_per_minute.amount_micros)
                .partial_cmp(&(b.vcpu, b.ram_gb, b.rate_per_minute.amount_micros))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|option| option.class_id.clone())
}

/// The classes a change would leave agents allowed to acquire without asking, planned before anything is asked or written.
struct ClassPlan {
    /// The classes allowed now (the smallest alone when the person named none).
    current: Vec<String>,
    /// The classes allowed after the change.
    target: Vec<String>,
    /// What the setting would hold: empty is "the smallest only" (unset).
    wanted: Vec<String>,
    /// The change adds a class the setting does not allow now: a raise.
    adds: bool,
}

fn plan_no_ask_classes(
    settings: &crate::config::UserAgentsConfig,
    classes: Vec<String>,
    capability: &MachineAllocationCapability,
) -> Result<ClassPlan, DaemonError> {
    let offered: Vec<&str> = capability.offers.iter().map(|o| o.class_id.as_str()).collect();
    let mut wanted: Vec<String> = Vec::new();
    for class in classes {
        if !offered.contains(&class.as_str()) {
            return Err(acquisition_error(
                "invalid_request",
                &format!("{class} is not a machine class the service offers (chariox.machines.options lists them)"),
            ));
        }
        if !wanted.contains(&class) {
            wanted.push(class);
        }
    }
    let smallest = smallest_class(capability);
    let current: Vec<String> = settings
        .offload_classes
        .clone()
        .unwrap_or_else(|| smallest.clone().into_iter().collect());
    let target: Vec<String> = if wanted.is_empty() {
        smallest.into_iter().collect()
    } else {
        wanted.clone()
    };
    let adds = !target.iter().all(|class| current.contains(class));
    Ok(ClassPlan {
        current,
        target,
        wanted,
        adds,
    })
}

/// The limits moved after the caller read them.
pub(crate) fn stale_limits() -> DaemonError {
    acquisition_error(
        "limits_changed",
        "The account limits changed since they were read; read them again before changing them",
    )
}

/// The service has no account limits.
pub(crate) fn no_account_limits() -> DaemonError {
    acquisition_error(
        "no_account_limits",
        "This kernel's connected service has no account limits",
    )
}

/// The no-ask settings changed while the person was deciding: the change was approved against settings that are gone.
pub(crate) fn stale_settings() -> DaemonError {
    acquisition_error(
        "limits_changed",
        "The limits for acquiring without asking changed while this was open; read them again before changing them",
    )
}
