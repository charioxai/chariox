//! What a user calls a kernel: the name the relay or Cloud directory shows for it
//! (the computer's name, `-2`, `-3` for further kernels on that computer), or its kernel id.
use chariox_relay::protocol::RelayKernelPresence;

use super::error::GatewayError;
use crate::local::GatewayKernel;

pub(super) fn label(kernel: &RelayKernelPresence) -> String {
    kernel
        .kernel_alias
        .clone()
        .or_else(|| kernel.relay_alias.clone())
        .unwrap_or_else(|| kernel.kernel_id.clone())
}

/// How this kernel names itself to its owner: the name of its computer.
pub(super) fn own_name(config: &crate::config::DaemonConfig) -> String {
    config
        .machine_name()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| config.host_machine_id.clone())
}

/// The name `chariox kernels` shows for `kernel`: the distinct name the relay or Cloud
/// directory gives it, else the computer's name (shared by every kernel on that computer).
pub(super) fn display_name(kernel: &RelayKernelPresence) -> String {
    kernel
        .kernel_alias
        .clone()
        .or_else(|| kernel.relay_alias.clone())
        .or_else(|| kernel.machine_alias.clone())
        .unwrap_or_else(|| kernel.kernel_id.clone())
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

/// The one kernel `name` means. A kernel id wins, then a kernel's own name; a computer
/// name shared by several kernels is `kernel_ambiguous` and lists every candidate with
/// the kernel id that picks it.
pub(super) fn find<'a>(
    all: &[&'a RelayKernelPresence],
    name: &str,
) -> Result<&'a RelayKernelPresence, GatewayError> {
    let by = |matches: Vec<&'a RelayKernelPresence>| match matches.as_slice() {
        [one] => Some(Ok(*one)),
        [] => None,
        several => Some(Err(GatewayError::Ambiguous {
            name: name.into(),
            candidates: several
                .iter()
                .map(|k| format!("{} (kernel id {})", display_name(k), k.kernel_id))
                .collect(),
        })),
    };
    let named = |names: fn(&RelayKernelPresence) -> [Option<&str>; 2]| {
        all.iter()
            .copied()
            .filter(|k| names(k).into_iter().flatten().any(|own| own == name))
            .collect::<Vec<_>>()
    };
    by(all
        .iter()
        .copied()
        .filter(|k| k.kernel_id == name)
        .collect())
    .or_else(|| {
        by(named(|k| {
            [k.kernel_alias.as_deref(), k.relay_alias.as_deref()]
        }))
    })
    .or_else(|| by(named(|k| [k.machine_alias.as_deref(), None])))
    .unwrap_or_else(|| Err(GatewayError::Unknown { name: name.into() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A registration names the kernel through the relay name only: `kernel_alias` is gone.
    fn kernel(id: &str, machine: &str, relay_alias: Option<&str>) -> RelayKernelPresence {
        RelayKernelPresence {
            kernel_id: id.into(),
            machine_id: format!("machine-id-{machine}"),
            machine_alias: Some(machine.into()),
            relay_alias: relay_alias.map(Into::into),
            kernel_alias: None,
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
    fn a_kernel_is_named_by_its_own_name_and_a_shared_computer_name_is_ambiguous() {
        let (main, second, solo) = (
            kernel("k-main", "b3", Some("b3")),
            kernel("k-second", "b3", Some("b3-2")),
            kernel("k-mac", "mac", Some("mac")),
        );
        let all = [&main, &second, &solo];
        assert_eq!(display_name(&main), "b3");
        assert_eq!(display_name(&second), "b3-2");
        assert_eq!(find(&all, "b3-2").unwrap().kernel_id, "k-second");
        assert_eq!(find(&all, "b3").unwrap().kernel_id, "k-main");
        assert_eq!(find(&all, "mac").unwrap().kernel_id, "k-mac");
        assert_eq!(find(&all, "k-second").unwrap().kernel_id, "k-second");
        assert_eq!(find(&all, "nope").unwrap_err().code(), "kernel_unknown");
        let unnamed = [&kernel("k-a", "pc", None), &kernel("k-b", "pc", None)];
        let ambiguous = find(&unnamed, "pc").unwrap_err();
        assert_eq!(ambiguous.code(), "kernel_ambiguous");
        assert!(ambiguous.message().contains("k-a") && ambiguous.message().contains("k-b"));
    }

    // Two kernels of one computer share its name as `machine_alias`; the relay's own names
    // for them (`pc`, `pc-2`) are what tell them apart.
    #[test]
    fn two_kernels_of_one_computer_are_shown_under_their_distinct_relay_names() {
        let (first, second) = (
            kernel("k-1", "pc", Some("pc")),
            kernel("k-2", "pc", Some("pc-2")),
        );
        assert_eq!(display_name(&first), "pc");
        assert_eq!(display_name(&second), "pc-2");
        assert_eq!(label(&second), "pc-2");
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
    fn a_kernel_names_itself_by_its_computer() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.host_machine_id = "machine-1".into();
        config.machine_name_for_tests = None;
        assert_eq!(own_name(&config), "machine-1");
        config.machine_name_for_tests = Some("mac".into());
        assert_eq!(own_name(&config), "mac");
    }
}
