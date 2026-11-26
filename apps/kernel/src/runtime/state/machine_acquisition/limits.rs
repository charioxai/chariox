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
}

impl AccountLimits {
    pub(crate) fn from_response(response: &crate::local::ControlPlaneResponse) -> Option<Self> {
        match response {
            crate::local::ControlPlaneResponse::Limits {
                machines_at_once,
                spend_per_day_micros,
                currency,
                version,
            } => Some(Self {
                values: AccountLimitValues {
                    machines_at_once: *machines_at_once,
                    spend_per_day_micros: *spend_per_day_micros,
                },
                currency: currency.clone(),
                version: *version,
            }),
            _ => None,
        }
    }

    fn reading(&self) -> AccountLimitsReading {
        AccountLimitsReading {
            machines_at_once: self.values.machines_at_once,
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

    /// The person's card for a raise: shown to every client of the Room, answered only by the person,
    /// never by the requester or its sub-agents. Returns the card's id once it is approved.
    async fn ask_limit_raise(
        &self,
        caller: &AgentCaller,
        scope: LimitScope,
        from: &AccountLimitValues,
        to: &AccountLimitValues,
        currency: &str,
    ) -> Result<String, DaemonError> {
        let (room, owner, agent) = (&caller.room, &caller.owner, caller.agent.as_deref());
        let id = format!(
            "{LIMITS_CARD_PREFIX}{:x}",
            sha2::Sha256::digest(
                format!(
                    "{room}\0{}\0{scope:?}\0{from:?}\0{to:?}",
                    agent.unwrap_or_default()
                )
                .as_bytes()
            )
        );
        let name = agent
            .and_then(|agent| self.owned.agent_store.get_agent(agent).ok())
            .map(|known| known.alias().unwrap_or(known.agent_ref()).to_owned())
            .unwrap_or_else(|| "An agent".into());
        let title = match scope {
            LimitScope::Account => format!("{name} asks to raise the account limits."),
            LimitScope::NoAsk => {
                format!("{name} asks to raise your limits for machines it acquires without asking.")
            }
        };
        let description = change_text(scope, from, to, currency);
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

    /// An agent's (or the owner's) change of the no-ask limits. A lower applies; a raise of an agent
    /// needs the person's `/sudo` or card. The owner's own change is the owner's.
    pub(super) async fn change_no_ask_limits(
        &self,
        caller: Option<AgentCaller>,
        machines_at_once: Option<u32>,
        spend_per_day_micros: Option<u64>,
        account: Option<&AccountLimits>,
    ) -> Result<(), DaemonError> {
        let settings = self.owned.config_projection.snapshot().user_config.agents;
        let current_machines = settings
            .offload_max_machines
            .unwrap_or(crate::config::DEFAULT_OFFLOAD_MAX_MACHINES);
        let from = AccountLimitValues {
            machines_at_once: current_machines,
            // No cap of its own is the largest value: any number is a lower.
            spend_per_day_micros: settings.offload_daily_cap_micros.unwrap_or(u64::MAX),
        };
        let to = AccountLimitValues {
            machines_at_once: machines_at_once.unwrap_or(from.machines_at_once),
            spend_per_day_micros: spend_per_day_micros.unwrap_or(from.spend_per_day_micros),
        };
        if from == to {
            return Ok(());
        }
        if let Some(caller) = caller {
            if raised(&from, &to) != RequestedRaise::default() {
                let currency = account.map_or("", |a| a.currency.as_str());
                self.raise_authority(&caller, LimitScope::NoAsk, &from, &to, currency)
                    .await?;
            }
        }
        if to.machines_at_once != from.machines_at_once {
            self.set_user_config_value(
                "agents.offload_max_machines".into(),
                to.machines_at_once.to_string(),
            )
            .await?;
        }
        if to.spend_per_day_micros != from.spend_per_day_micros {
            self.set_user_config_value(
                "agents.offload_daily_cap_micros".into(),
                to.spend_per_day_micros.to_string(),
            )
            .await?;
        }
        Ok(())
    }
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
