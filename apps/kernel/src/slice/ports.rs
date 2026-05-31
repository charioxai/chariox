use std::net::TcpListener;

use crate::slice::SliceRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LocalDockerSlicePorts {
    pub(super) codex: u16,
    pub(super) opencode: u16,
    pub(super) kernel: u16,
    pub(super) mcp: u16,
    pub(super) relay: u16,
    pub(super) novnc: u16,
}

impl LocalDockerSlicePorts {
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
        }
    }

    pub(super) fn codex_range(self) -> String {
        let start = 43362_u16.saturating_add(self.ordinal_offset());
        format!("{start}-{}", start.saturating_add(19))
    }

    pub(super) fn opencode_range(self) -> String {
        let start = 43150_u16.saturating_add(self.ordinal_offset());
        format!("{start}-{}", start.saturating_add(19))
    }

    fn ordinal_offset(self) -> u16 {
        self.kernel.saturating_sub(53119).saturating_mul(20)
    }

    fn published_ports(self) -> Vec<u16> {
        let offset = self.ordinal_offset();
        let mut ports = vec![
            self.codex,
            self.opencode,
            self.kernel,
            self.relay,
            self.novnc,
        ];
        ports.extend(43362_u16.saturating_add(offset)..=43381_u16.saturating_add(offset));
        ports.extend(43150_u16.saturating_add(offset)..=43169_u16.saturating_add(offset));
        ports.sort_unstable();
        ports.dedup();
        ports
    }
}

pub(super) fn busy_published_ports_for_slice(record: &SliceRecord) -> Vec<u16> {
    LocalDockerSlicePorts::for_slice_id(&record.id)
        .published_ports()
        .into_iter()
        .filter(|port| TcpListener::bind(("127.0.0.1", *port)).is_err())
        .collect()
}
