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
use crate::runtime::state::{LimitGrant, RequestedRaise};

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
    let whole = micros / 1_000_000;
    let fraction = micros % 1_000_000;
    if micros % 10_000 == 0 {
        format!("{whole}.{:02} {currency}", fraction / 10_000)
    } else {
        let fraction = format!("{fraction:06}");
        format!("{whole}.{} {currency}", fraction.trim_end_matches('0'))
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

fn missing_limit_authority(scope: LimitScope, raise: &RequestedRaise, currency: &str, classes: Option<&[String]>) -> DaemonError {
    let step = LimitGrant::prompt_for(scope, raise, currency, classes).map_or_else(
        || {
            let scope_name = match scope { LimitScope::Account => "account", LimitScope::NoAsk => "without asking" };
            let mut values = Vec::new();
            if let Some(machines) = raise.machines_at_once { values.push(format!("{machines} machines at once")); }
            if let Some(spend) = raise.spend_per_day_micros { values.push(format!("{} a day", amount_text(spend, currency))); }
            if let Some(classes) = classes { values.push(format!("classes {}", classes.join(", "))); }
            format!("Have the Room owner set their {scope_name} limits directly to {}. The existing named /sudo amount grammar cannot represent this exact request; nothing is rounded or widened.", values.join(" and "))
        },
        |prompt| format!("Have the Room owner submit `/sudo {prompt}` for this Room and agent, then retry this exact change."),
    );
    acquisition_error("owner_action_required", &format!("The person's named authority is required to raise these limits. Nothing changed. {step}"))
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

    /// Only the person's parsed, named `/sudo` grant authorizes a raise.
    async fn raise_authority(
        &self,
        caller: &AgentCaller,
        scope: LimitScope,
        from: &AccountLimitValues,
        to: &AccountLimitValues,
        currency: &str,
    ) -> Result<(LimitApprovalKind, String), DaemonError> {
        if let Some((entry, grant)) = self.sudo_limit_grant(caller) {
            if grant.covers_change(scope, &raised(from, to), currency, None) {
                return Ok((LimitApprovalKind::Sudo, entry));
            }
        }
        Err(missing_limit_authority(scope, &raised(from, to), currency, None))
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
    /// planned and authorized against, so a refusal leaves nothing half done and an edit made meanwhile is never lost.
    /// A lower applies; an agent's raise needs the person's exact named `/sudo`. The owner's own change is the owner's.
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
                let currency = account.map_or("", |account| account.currency.as_str());
                let authorized = self.sudo_limit_grant(caller).is_some_and(|(_, grant)| {
                    grant.covers_change(LimitScope::NoAsk, &raised(&from, &to), currency, Some(&plan.target))
                });
                if !authorized {
                    return Err(missing_limit_authority(LimitScope::NoAsk, &raised(&from, &to), currency, Some(&plan.target)));
                }
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

#[cfg(test)]
mod named_authority_refusal_tests {
    use super::*;

    #[test]
    fn missing_numeric_and_class_authority_names_one_complete_owner_step() {
        let classes = vec!["small".to_owned(), "big".to_owned()];
        let wanted = RequestedRaise { machines_at_once: Some(4), spend_per_day_micros: Some(15_000_000) };
        let error = missing_limit_authority(LimitScope::NoAsk, &wanted, "EUR", Some(&classes));
        let DaemonError::MachineRequest { code, message } = error else { panic!("expected the existing machine refusal") };
        assert_eq!(code, "owner_action_required");
        assert!(message.contains(r#"/sudo raise no-ask limits to 4 machines and 15.00 EUR a day and classes ["small","big"]"#), "{message}");
        assert!(message.contains("Nothing changed"));
    }

    #[test]
    fn an_unrepresentable_exact_amount_is_never_rounded_into_a_broader_grant() {
        let error = missing_limit_authority(LimitScope::Account, &RequestedRaise { machines_at_once: None, spend_per_day_micros: Some(15_000_001) }, "EUR", None);
        let DaemonError::MachineRequest { code, message } = error else { panic!("expected the existing machine refusal") };
        assert_eq!(code, "owner_action_required");
        assert!(message.contains("15.000001 EUR"), "{message}");
        assert!(!message.contains("/sudo raise"), "no unusable or rounded owner command: {message}");
    }
}
