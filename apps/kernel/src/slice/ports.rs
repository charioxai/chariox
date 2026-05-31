use std::net::TcpListener;

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

    fn dynamic_candidate(index: u16) -> Self {
        let range_offset = index.saturating_mul(20);
        Self {
            codex: 44000_u16.saturating_add(index),
            opencode: 44300_u16.saturating_add(index),
            kernel: 44600_u16.saturating_add(index),
            mcp: 44900_u16.saturating_add(index),
            relay: 45200_u16.saturating_add(index),
            novnc: 45500_u16.saturating_add(index),
            codex_range_start: 46000_u16.saturating_add(range_offset),
            opencode_range_start: 51200_u16.saturating_add(range_offset),
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
    let reserved = records
        .values()
        .filter(|record| record.backend == crate::slice::SliceBackendKind::LocalDocker)
        .flat_map(|record| LocalDockerSlicePorts::for_record(record).published_ports())
        .collect::<BTreeSet<_>>();
    for index in 0..MAX_DYNAMIC_SLICE_PORT_SETS {
        let ports = LocalDockerSlicePorts::dynamic_candidate(index);
        let published = ports.published_ports();
        if published.iter().any(|port| reserved.contains(port)) {
            continue;
        }
        if published.iter().any(|port| port_is_busy(*port)) {
            continue;
        }
        return Ok(ports.to_assignment());
    }
    Err(DaemonError::LocalTransport {
        operation: "slice.local_docker.ports",
        message: format!(
            "no free local Docker slice port set found after scanning {MAX_DYNAMIC_SLICE_PORT_SETS} candidates"
        ),
    })
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
