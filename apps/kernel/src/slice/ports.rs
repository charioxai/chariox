use std::net::TcpListener;
use std::ops::RangeInclusive;

use std::collections::{BTreeMap, BTreeSet};

use crate::error::DaemonError;
use crate::slice::{SliceLocalDockerPorts, SliceRecord};

const MAX_DYNAMIC_SLICE_PORT_SETS: u16 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LocalDockerSlicePorts {
    pub(super) codex: u16,
    pub(super) opencode: u16,
    pub(super) kernel: u16,
    pub(super) mcp: u16,
    pub(super) relay: u16,
    pub(super) novnc: u16,
    pub(super) codex_range_start: u16,
    pub(super) opencode_range_start: u16,
}

impl LocalDockerSlicePorts {
    pub(super) fn from_assignment(ports: SliceLocalDockerPorts) -> Self {
        Self {
            codex: ports.codex,
            opencode: ports.opencode,
            kernel: ports.kernel,
            mcp: ports.mcp,
            relay: ports.relay,
            novnc: ports.novnc,
            codex_range_start: ports.codex_range_start,
            opencode_range_start: ports.opencode_range_start,
        }
    }

    pub(super) fn for_record(record: &SliceRecord) -> Self {
        record
            .local_docker_ports
            .map(Self::from_assignment)
            .unwrap_or_else(|| Self::for_slice_id(&record.id))
    }

    pub(super) fn for_slice_id(slice_id: &str) -> Self {
        let ordinal = slice_id
            .strip_prefix("slice-")
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(1)
            .saturating_sub(1);
        Self {
            codex: 43252_u16.saturating_add(ordinal),
            opencode: 43140_u16.saturating_add(ordinal),
            kernel: 53119_u16.saturating_add(ordinal),
            mcp: 53120_u16.saturating_add(ordinal),
            relay: 53130_u16.saturating_add(ordinal),
            novnc: 16080_u16.saturating_add(ordinal),
            codex_range_start: 43362_u16.saturating_add(ordinal.saturating_mul(20)),
            opencode_range_start: 43150_u16.saturating_add(ordinal.saturating_mul(20)),
        }
    }

    pub(super) fn codex_range(self) -> String {
        let start = self.codex_range_start;
        format!("{start}-{}", start.saturating_add(19))
    }

    pub(super) fn opencode_range(self) -> String {
        let start = self.opencode_range_start;
        format!("{start}-{}", start.saturating_add(19))
    }

    // New port sets lie in 20000-32319, below every default ephemeral range:
    // Linux 32768-60999 (also inside Colima's VM, where Docker publishes on
    // macOS) and macOS 49152-65535. The host hands ephemeral ports to outgoing
    // connections at any time, so a published port inside that range can be
    // taken between allocation and `docker start`.
    fn dynamic_candidate(index: u16) -> Self {
        let range_offset = index.saturating_mul(20);
        Self {
            codex: 20000_u16.saturating_add(index),
            opencode: 20300_u16.saturating_add(index),
            kernel: 20600_u16.saturating_add(index),
            mcp: 20900_u16.saturating_add(index),
            relay: 21200_u16.saturating_add(index),
            novnc: 21500_u16.saturating_add(index),
            codex_range_start: 22000_u16.saturating_add(range_offset),
            opencode_range_start: 27200_u16.saturating_add(range_offset),
        }
    }

    fn to_assignment(self) -> SliceLocalDockerPorts {
        SliceLocalDockerPorts {
            codex: self.codex,
            opencode: self.opencode,
            kernel: self.kernel,
            mcp: self.mcp,
            relay: self.relay,
            novnc: self.novnc,
            codex_range_start: self.codex_range_start,
            opencode_range_start: self.opencode_range_start,
        }
    }

    fn published_ports(self) -> Vec<u16> {
        let mut ports = vec![
            self.codex,
            self.opencode,
            self.kernel,
            self.relay,
            self.novnc,
        ];
        ports.extend(self.codex_range_start..=self.codex_range_start.saturating_add(19));
        ports.extend(self.opencode_range_start..=self.opencode_range_start.saturating_add(19));
        ports.sort_unstable();
        ports.dedup();
        ports
    }
}

pub(super) fn allocate_local_docker_ports_for_slice(
    records: &BTreeMap<String, SliceRecord>,
) -> Result<SliceLocalDockerPorts, DaemonError> {
    allocate_ports(records, host_ephemeral_port_range().as_ref(), port_is_busy)
}

// Existing slices keep their persisted ports: their containers' bindings, the
// display endpoint and Room forwarding depend on them. Only new sets move.
fn allocate_ports(
    records: &BTreeMap<String, SliceRecord>,
    ephemeral: Option<&RangeInclusive<u16>>,
    busy: impl Fn(u16) -> bool,
) -> Result<SliceLocalDockerPorts, DaemonError> {
    let reserved = records
        .values()
        .filter(|record| record.backend == crate::slice::SliceBackendKind::LocalDocker)
        .flat_map(|record| LocalDockerSlicePorts::for_record(record).published_ports())
        .collect::<BTreeSet<_>>();
    // Prefer a set outside the host's ephemeral range. If a configured range
    // covers every candidate, any free set is still better than none.
    for avoid_ephemeral in [true, false] {
        for index in 0..MAX_DYNAMIC_SLICE_PORT_SETS {
            let ports = LocalDockerSlicePorts::dynamic_candidate(index);
            let published = ports.published_ports();
            if avoid_ephemeral
                && ephemeral.is_some_and(|range| published.iter().any(|port| range.contains(port)))
            {
                continue;
            }
            if published.iter().any(|port| reserved.contains(port)) {
                continue;
            }
            if published.iter().any(|port| busy(*port)) {
                continue;
            }
            return Ok(ports.to_assignment());
        }
    }
    Err(DaemonError::LocalTransport {
        operation: "slice.local_docker.ports",
        message: format!(
            "no free local Docker slice port set found after scanning {MAX_DYNAMIC_SLICE_PORT_SETS} candidates"
        ),
    })
}

// Docker publishes in the host network namespace on Linux. Elsewhere it runs
// in a VM whose range the kernel cannot read; the candidates avoid its default.
#[cfg(target_os = "linux")]
fn host_ephemeral_port_range() -> Option<RangeInclusive<u16>> {
    parse_port_range(&std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range").ok()?)
}

#[cfg(not(target_os = "linux"))]
fn host_ephemeral_port_range() -> Option<RangeInclusive<u16>> {
    None
}

fn parse_port_range(text: &str) -> Option<RangeInclusive<u16>> {
    let mut bounds = text.split_whitespace().map(str::parse::<u16>);
    match (bounds.next(), bounds.next(), bounds.next()) {
        (Some(Ok(low)), Some(Ok(high)), None) if low <= high => Some(low..=high),
        _ => None,
    }
}

pub(super) fn busy_published_ports_for_slice(record: &SliceRecord) -> Vec<u16> {
    LocalDockerSlicePorts::for_record(record)
        .published_ports()
        .into_iter()
        .filter(|port| port_is_busy(*port))
        .collect()
}

fn port_is_busy(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_err()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_candidate() -> impl Iterator<Item = LocalDockerSlicePorts> {
        (0..MAX_DYNAMIC_SLICE_PORT_SETS).map(LocalDockerSlicePorts::dynamic_candidate)
    }

    #[test]
    fn new_port_sets_stay_below_every_default_ephemeral_range() {
        let mut seen = BTreeSet::new();
        for ports in every_candidate() {
            for port in ports.published_ports().into_iter().chain([ports.mcp]) {
                assert!(
                    (1024..32768).contains(&port),
                    "port {port} is privileged or inside a default ephemeral range"
                );
                assert!(seen.insert(port), "port {port} is shared by two port sets");
            }
        }
        assert_eq!(seen.len(), usize::from(MAX_DYNAMIC_SLICE_PORT_SETS) * 46);
    }

    #[test]
    fn allocation_skips_sets_inside_the_configured_ephemeral_range() {
        let ephemeral = 20000..=20009;
        let ports = allocate_ports(&BTreeMap::new(), Some(&ephemeral), |_| false)
            .expect("a set outside the ephemeral range should be free");
        assert_eq!(
            ports,
            LocalDockerSlicePorts::dynamic_candidate(10).to_assignment()
        );
    }

    #[test]
    fn allocation_falls_back_to_a_free_set_when_every_set_is_ephemeral() {
        let ephemeral = 1024..=65535;
        let ports = allocate_ports(&BTreeMap::new(), Some(&ephemeral), |_| false)
            .expect("a free set should still be allocated");
        assert_eq!(
            ports,
            LocalDockerSlicePorts::dynamic_candidate(0).to_assignment()
        );
    }

    #[test]
    fn a_taken_port_moves_allocation_to_the_next_free_set() {
        let first = LocalDockerSlicePorts::dynamic_candidate(0);
        let taken = first.opencode_range_start + 7;
        let ports = allocate_ports(&BTreeMap::new(), Some(&(32768..=60999)), |port| {
            port == taken
        })
        .expect("the next set should be free");
        assert_eq!(
            ports,
            LocalDockerSlicePorts::dynamic_candidate(1).to_assignment()
        );
    }

    #[test]
    fn ephemeral_port_range_parses_the_kernel_format_only() {
        assert_eq!(parse_port_range("32768\t60999\n"), Some(32768..=60999));
        assert_eq!(parse_port_range("1024 65535"), Some(1024..=65535));
        for invalid in ["", "32768", "60999 32768", "1 2 3", "low high", "1 70000"] {
            assert_eq!(parse_port_range(invalid), None, "{invalid:?}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn allocated_ports_avoid_the_host_ephemeral_range() {
        let Some(ephemeral) = host_ephemeral_port_range() else {
            return;
        };
        if every_candidate().all(|ports| {
            ports
                .published_ports()
                .iter()
                .any(|port| ephemeral.contains(port))
        }) {
            return;
        }
        let ports = LocalDockerSlicePorts::from_assignment(
            allocate_local_docker_ports_for_slice(&BTreeMap::new())
                .expect("a free port set should exist"),
        );
        for port in ports.published_ports() {
            assert!(
                !ephemeral.contains(&port),
                "port {port} is inside the host ephemeral range {ephemeral:?}"
            );
        }
    }
}
