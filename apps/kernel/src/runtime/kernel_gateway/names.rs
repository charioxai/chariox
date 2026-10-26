//! What a user calls a kernel. A machine can run several of the user's kernels, so
//! the name is the machine's when it is the only kernel there and `<machine>/<label>`
//! otherwise; the kernel id and its aliases are accepted too.
use chariox_relay::protocol::RelayKernelPresence;

use super::error::GatewayError;
use crate::local::GatewayKernel;

fn machine_name(kernel: &RelayKernelPresence) -> String {
    kernel
        .machine_alias
        .clone()
        .unwrap_or_else(|| kernel.machine_id.clone())
}

pub(super) fn label(kernel: &RelayKernelPresence) -> String {
    kernel
        .kernel_alias
        .clone()
        .or_else(|| kernel.relay_alias.clone())
        .unwrap_or_else(|| kernel.kernel_id.clone())
}

/// `<machine>/<alias>`, or the machine alone when the kernel has no alias of its own.
pub(super) fn machine_and_alias(machine: &str, alias: Option<&str>) -> String {
    match alias.map(str::trim) {
        Some(alias) if !alias.is_empty() && alias != machine => format!("{machine}/{alias}"),
        _ => machine.to_string(),
    }
}

/// How this kernel names itself to its owner.
pub(super) fn own_name(config: &crate::config::DaemonConfig) -> String {
    let machine = config
        .host_machine_alias
        .clone()
        .filter(|alias| !alias.trim().is_empty())
        .unwrap_or_else(|| config.host_machine_id.clone());
    machine_and_alias(&machine, config.daemon_alias.as_deref())
}

fn qualified(kernel: &RelayKernelPresence) -> String {
    format!("{}/{}", machine_name(kernel), label(kernel))
}

/// The name `chariox kernels` shows for `kernel`, among all the user's known kernels.
pub(super) fn display_name(kernel: &RelayKernelPresence, all: &[&RelayKernelPresence]) -> String {
    let shares_machine = all
        .iter()
        .any(|other| other.machine_id == kernel.machine_id && other.kernel_id != kernel.kernel_id);
    if shares_machine {
        qualified(kernel)
    } else {
        kernel
            .machine_alias
            .clone()
            .or_else(|| kernel.kernel_alias.clone())
            .or_else(|| kernel.relay_alias.clone())
            .unwrap_or_else(|| kernel.kernel_id.clone())
    }
}

/// The list `chariox kernels` prints: the kernels of one machine together, by label.
pub(super) fn grouped(mut kernels: Vec<GatewayKernel>) -> Vec<GatewayKernel> {
    kernels.sort_by(|a, b| {
        let machine = |k: &GatewayKernel| {
            k.machine_alias
                .clone()
                .unwrap_or_else(|| k.machine_id.clone())
        };
        (machine(a), &a.label).cmp(&(machine(b), &b.label))
    });
    kernels
}

/// The one kernel `name` means. A kernel id or a `<machine>/<label>` name wins; a
/// machine name or an alias shared by several kernels is `kernel_ambiguous` and lists
/// every candidate with the name that picks it.
pub(super) fn find<'a>(
    all: &[&'a RelayKernelPresence],
    name: &str,
) -> Result<&'a RelayKernelPresence, GatewayError> {
    let exact = |matches: Vec<&'a RelayKernelPresence>| match matches.as_slice() {
        [one] => Some(*one),
        _ => None,
    };
    if let Some(one) = exact(
        all.iter()
            .copied()
            .filter(|k| k.kernel_id == name)
            .collect(),
    ) {
        return Ok(one);
    }
    if let Some(one) = exact(
        all.iter()
            .copied()
            .filter(|k| qualified(k) == name)
            .collect(),
    ) {
        return Ok(one);
    }
    let matches: Vec<&RelayKernelPresence> = all
        .iter()
        .copied()
        .filter(|k| {
            [&k.kernel_alias, &k.relay_alias, &k.machine_alias]
                .into_iter()
                .flatten()
                .any(|alias| alias == name)
        })
        .collect();
    match matches.as_slice() {
        [] => Err(GatewayError::Unknown { name: name.into() }),
        [one] => Ok(one),
        several => Err(GatewayError::Ambiguous {
            name: name.into(),
            candidates: several
                .iter()
                .map(|k| format!("{} (kernel id {})", qualified(k), k.kernel_id))
                .collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kernel(id: &str, machine: &str, alias: Option<&str>) -> RelayKernelPresence {
        RelayKernelPresence {
            kernel_id: id.into(),
            machine_id: format!("machine-id-{machine}"),
            machine_alias: Some(machine.into()),
            relay_alias: None,
            kernel_alias: alias.map(Into::into),
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
            capabilities: Vec::new(),
            accepting_remote_leases: false,
            leased_agent_count: 0,
            local_session_count: 0,
            public_key: format!("key-{id}"),
        }
    }

    #[test]
    fn a_machine_with_two_kernels_needs_the_label_and_one_kernel_needs_only_the_machine() {
        let (main, dev, solo) = (
            kernel("k-main", "b3", Some("main")),
            kernel("k-dev", "b3", Some("dev")),
            kernel("k-mac", "mac", None),
        );
        let all = [&main, &dev, &solo];
        assert_eq!(display_name(&main, &all), "b3/main");
        assert_eq!(display_name(&dev, &all), "b3/dev");
        assert_eq!(display_name(&solo, &all), "mac");
        assert_eq!(find(&all, "b3/dev").unwrap().kernel_id, "k-dev");
        assert_eq!(find(&all, "mac").unwrap().kernel_id, "k-mac");
        assert_eq!(find(&all, "k-main").unwrap().kernel_id, "k-main");
        let ambiguous = find(&all, "b3").unwrap_err();
        assert_eq!(ambiguous.code(), "kernel_ambiguous");
        let message = ambiguous.message();
        assert!(
            message.contains("b3/main") && message.contains("b3/dev"),
            "{message}"
        );
        assert_eq!(find(&all, "nope").unwrap_err().code(), "kernel_unknown");
    }

    #[test]
    fn the_list_keeps_the_kernels_of_one_machine_together() {
        let entry = |machine: &str, label: &str| GatewayKernel {
            name: format!("{machine}/{label}"),
            kernel_id: format!("k-{machine}-{label}"),
            label: label.into(),
            machine_id: format!("id-{machine}"),
            machine_alias: Some(machine.into()),
            status: crate::local::GatewayKernelStatus::Online,
            last_seen_ms: 0,
            gateway_ready: true,
            via: crate::local::GatewayKernelVia::Hosted,
        };
        let listed = grouped(vec![
            entry("mac", "main"),
            entry("b3", "main"),
            entry("mac", "dev"),
            entry("b3", "dev"),
        ]);
        let order: Vec<&str> = listed.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(order, ["b3/dev", "b3/main", "mac/dev", "mac/main"]);
    }

    #[test]
    fn a_kernel_names_itself_by_its_machine_and_its_alias() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.host_machine_id = "machine-1".into();
        assert_eq!(own_name(&config), "machine-1");
        config.host_machine_alias = Some("mac".into());
        assert_eq!(own_name(&config), "mac");
        config.daemon_alias = Some(" dev ".into());
        assert_eq!(own_name(&config), "mac/dev");
        config.daemon_alias = Some("mac".into());
        assert_eq!(own_name(&config), "mac");
    }
}
