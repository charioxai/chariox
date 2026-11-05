//! An output destination that can still identify its volume after the writer exits.
use super::{disk_room, statvfs, Disk, Volume};
use crate::runtime::process_resources::disk_use::VolumeWrites;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(in crate::runtime::process_resources) struct ReceivingVolume {
    pub path: PathBuf,
    pub id: u64,
}

impl ReceivingVolume {
    pub(in crate::runtime::process_resources) fn capture(output: &VolumeWrites) -> Option<Self> {
        // A sampler path can be /proc/<pid>/fd/<n>. Only its resolved destination
        // survives that descriptor; another mount namespace may make it unknown.
        let target = std::fs::canonicalize(&output.path).ok()?;
        let path = target.parent()?.to_path_buf();
        let space = statvfs(&path)?;
        (space.id == output.id).then_some(Self {
            path,
            id: output.id,
        })
    }

    pub(in crate::runtime::process_resources) fn disk(&self, volume: &Volume) -> Option<Disk> {
        // Removed build outputs can be measured through their nearest surviving
        // ancestor, but a removed mount or replaced destination is not that volume.
        self.path
            .ancestors()
            .find_map(|path| disk_room(volume, path))
            .filter(|disk| disk.id == self.id)
    }
}
