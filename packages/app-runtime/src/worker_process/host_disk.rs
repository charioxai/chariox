//! Host disk headroom required by App storage. Refused preparation carries
//! measured free and required bytes so the owner can recover space.

/// Space kept free on the filesystem holding App storage, for everything else.
pub const HOST_RESERVE_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Not enough free disk space for App storage. `needed` is the reserve plus
/// the bytes storage has promised Apps but the filesystem has not allocated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostDiskSpace {
    pub free: u64,
    pub needed: u64,
}

impl HostDiskSpace {
    /// The storage rule: `free` bytes must cover the reserve and every
    /// promised but unallocated byte (`unallocated`).
    pub fn check(free: u64, unallocated: u64) -> Result<(), Self> {
        let needed = HOST_RESERVE_BYTES.saturating_add(unallocated);
        if free < needed {
            Err(Self { free, needed })
        } else {
            Ok(())
        }
    }

    /// Bytes to free before storage can promise again.
    pub fn shortfall(&self) -> u64 {
        self.needed.saturating_sub(self.free)
    }

    /// For the owner: what is free, what Apps need, and how much to free.
    pub fn message(&self) -> String {
        format!(
            "Not enough free disk space: Apps need {} free on the host ({} free). Free at least {}.",
            gib(self.needed, true),
            gib(self.free, false),
            gib(self.shortfall(), true)
        )
    }
}

/// Bytes in GiB with one decimal; `up` rounds up, so freeing the amount
/// shown is always enough.
fn gib(bytes: u64, up: bool) -> String {
    const GIB: u128 = 1024 * 1024 * 1024;
    let scaled = u128::from(bytes) * 10;
    let tenths = if up {
        scaled.div_ceil(GIB)
    } else {
        scaled / GIB
    };
    if tenths % 10 == 0 {
        format!("{} GiB", tenths / 10)
    } else {
        format!("{}.{} GiB", tenths / 10, tenths % 10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn the_reserve_covers_promised_unallocated_bytes() {
        assert_eq!(HostDiskSpace::check(HOST_RESERVE_BYTES, 0), Ok(()));
        assert_eq!(
            HostDiskSpace::check(HOST_RESERVE_BYTES - 1, 0),
            Err(HostDiskSpace {
                free: HOST_RESERVE_BYTES - 1,
                needed: HOST_RESERVE_BYTES,
            })
        );
        let promised = 576 * 1024 * 1024;
        assert_eq!(
            HostDiskSpace::check(HOST_RESERVE_BYTES + promised, promised),
            Ok(())
        );
        assert_eq!(
            HostDiskSpace::check(HOST_RESERVE_BYTES, promised)
                .unwrap_err()
                .shortfall(),
            promised
        );
        assert_eq!(
            HostDiskSpace::check(0, u64::MAX).unwrap_err().needed,
            u64::MAX
        );
    }

    #[test]
    fn the_message_says_what_is_free_and_rounds_what_to_free_up() {
        // The macOS drill: 7.56 GiB free with every image allocated.
        let short = HostDiskSpace::check(7 * GIB + 56 * GIB / 100, 0).unwrap_err();
        assert_eq!(
            short.message(),
            "Not enough free disk space: Apps need 8 GiB free on the host (7.5 GiB free). \
             Free at least 0.5 GiB."
        );
        let new_install = HostDiskSpace::check(8 * GIB, 576 * 1024 * 1024).unwrap_err();
        assert_eq!(
            new_install.message(),
            "Not enough free disk space: Apps need 8.6 GiB free on the host (8 GiB free). \
             Free at least 0.6 GiB."
        );
        let empty = HostDiskSpace::check(0, 0).unwrap_err();
        assert_eq!(
            empty.message(),
            "Not enough free disk space: Apps need 8 GiB free on the host (0 GiB free). \
             Free at least 8 GiB."
        );
    }
}
