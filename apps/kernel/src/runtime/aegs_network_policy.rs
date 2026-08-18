use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Clone, Copy)]
struct IpPrefix<T> {
    network: T,
    prefix_len: u8,
}

impl IpPrefix<Ipv4Addr> {
    const fn new(network: Ipv4Addr, prefix_len: u8) -> Self {
        Self {
            network,
            prefix_len,
        }
    }

    fn contains(self, address: Ipv4Addr) -> bool {
        let host_bits = 32_u32.saturating_sub(u32::from(self.prefix_len));
        let mask = u32::MAX.checked_shl(host_bits).unwrap_or(0);
        u32::from(address) & mask == u32::from(self.network) & mask
    }
}

impl IpPrefix<Ipv6Addr> {
    const fn new(network: Ipv6Addr, prefix_len: u8) -> Self {
        Self {
            network,
            prefix_len,
        }
    }

    fn contains(self, address: Ipv6Addr) -> bool {
        let host_bits = 128_u32.saturating_sub(u32::from(self.prefix_len));
        let mask = u128::MAX.checked_shl(host_bits).unwrap_or(0);
        u128::from(address) & mask == u128::from(self.network) & mask
    }
}

const fn v4(a: u8, b: u8, c: u8, d: u8, prefix_len: u8) -> IpPrefix<Ipv4Addr> {
    IpPrefix::<Ipv4Addr>::new(Ipv4Addr::new(a, b, c, d), prefix_len)
}

#[allow(clippy::too_many_arguments)]
const fn v6(
    a: u16,
    b: u16,
    c: u16,
    d: u16,
    e: u16,
    f: u16,
    g: u16,
    h: u16,
    prefix_len: u8,
) -> IpPrefix<Ipv6Addr> {
    IpPrefix::<Ipv6Addr>::new(Ipv6Addr::new(a, b, c, d, e, f, g, h), prefix_len)
}

// Snapshot of entries whose Globally Reachable field is false (or whose
// deprecated allocation has no reachability grant) in the IANA IPv4
// Special-Purpose Address Space registry, last updated 2025-10-09. Multicast
// is also rejected because an AEGS HTTP destination must be unicast.
// https://www.iana.org/assignments/iana-ipv4-special-registry/
const NON_GLOBAL_IPV4_PREFIXES: &[IpPrefix<Ipv4Addr>] = &[
    v4(0, 0, 0, 0, 8),
    v4(10, 0, 0, 0, 8),
    v4(100, 64, 0, 0, 10),
    v4(127, 0, 0, 0, 8),
    v4(169, 254, 0, 0, 16),
    v4(172, 16, 0, 0, 12),
    v4(192, 0, 0, 0, 24),
    v4(192, 0, 2, 0, 24),
    v4(192, 88, 99, 0, 24),
    v4(192, 168, 0, 0, 16),
    v4(198, 18, 0, 0, 15),
    v4(198, 51, 100, 0, 24),
    v4(203, 0, 113, 0, 24),
    v4(224, 0, 0, 0, 4),
    v4(240, 0, 0, 0, 4),
];

// More-specific globally reachable entries inside 192.0.0.0/24.
const GLOBAL_IPV4_EXCEPTIONS: &[IpPrefix<Ipv4Addr>] =
    &[v4(192, 0, 0, 9, 32), v4(192, 0, 0, 10, 32)];

// Snapshot of non-globally-reachable entries in the IANA IPv6
// Special-Purpose Address Space registry, last updated 2025-10-09. The
// explicit exceptions below are the more-specific entries that IANA marks as
// globally reachable inside 2001::/23. 6to4/Teredo entries with an N/A global
// reachability value are rejected. Multicast is rejected for HTTP unicast.
// https://www.iana.org/assignments/iana-ipv6-special-registry/
const NON_GLOBAL_IPV6_PREFIXES: &[IpPrefix<Ipv6Addr>] = &[
    v6(0, 0, 0, 0, 0, 0, 0, 0, 128),
    v6(0, 0, 0, 0, 0, 0, 0, 1, 128),
    v6(0, 0, 0, 0, 0, 0xffff, 0, 0, 96),
    v6(0x0064, 0xff9b, 0x0001, 0, 0, 0, 0, 0, 48),
    v6(0x0100, 0, 0, 0, 0, 0, 0, 0, 64),
    v6(0x0100, 0, 0, 1, 0, 0, 0, 0, 64),
    v6(0x2001, 0, 0, 0, 0, 0, 0, 0, 23),
    v6(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0, 32),
    v6(0x2002, 0, 0, 0, 0, 0, 0, 0, 16),
    v6(0x3fff, 0, 0, 0, 0, 0, 0, 0, 20),
    v6(0x5f00, 0, 0, 0, 0, 0, 0, 0, 16),
    v6(0xfc00, 0, 0, 0, 0, 0, 0, 0, 7),
    v6(0xfe80, 0, 0, 0, 0, 0, 0, 0, 10),
    v6(0xff00, 0, 0, 0, 0, 0, 0, 0, 8),
];

const GLOBAL_IPV6_EXCEPTIONS: &[IpPrefix<Ipv6Addr>] = &[
    v6(0x2001, 0x0001, 0, 0, 0, 0, 0, 1, 128),
    v6(0x2001, 0x0001, 0, 0, 0, 0, 0, 2, 128),
    v6(0x2001, 0x0001, 0, 0, 0, 0, 0, 3, 128),
    v6(0x2001, 0x0003, 0, 0, 0, 0, 0, 0, 32),
    v6(0x2001, 0x0004, 0x0112, 0, 0, 0, 0, 0, 48),
    v6(0x2001, 0x0020, 0, 0, 0, 0, 0, 0, 28),
    v6(0x2001, 0x0030, 0, 0, 0, 0, 0, 0, 28),
];

// IANA currently allocates ordinary IPv6 global unicast destinations from
// 2000::/3; the separately registered well-known translation prefix is also
// globally reachable. The rest of the address space remains reserved.
// https://www.iana.org/assignments/ipv6-address-space/
const IPV6_GLOBAL_UNICAST: IpPrefix<Ipv6Addr> = v6(0x2000, 0, 0, 0, 0, 0, 0, 0, 3);
const IPV6_WELL_KNOWN_TRANSLATION: IpPrefix<Ipv6Addr> = v6(0x0064, 0xff9b, 0, 0, 0, 0, 0, 0, 96);

pub(crate) fn is_globally_reachable_aegs_destination(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            if GLOBAL_IPV4_EXCEPTIONS
                .iter()
                .any(|prefix| prefix.contains(address))
            {
                return true;
            }
            !NON_GLOBAL_IPV4_PREFIXES
                .iter()
                .any(|prefix| prefix.contains(address))
        }
        IpAddr::V6(address) => {
            if GLOBAL_IPV6_EXCEPTIONS
                .iter()
                .any(|prefix| prefix.contains(address))
            {
                return true;
            }
            if NON_GLOBAL_IPV6_PREFIXES
                .iter()
                .any(|prefix| prefix.contains(address))
            {
                return false;
            }
            IPV6_GLOBAL_UNICAST.contains(address) || IPV6_WELL_KNOWN_TRANSLATION.contains(address)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last_v4(prefix: IpPrefix<Ipv4Addr>) -> Ipv4Addr {
        Ipv4Addr::from(
            u32::from(prefix.network) | u32::MAX.checked_shr(prefix.prefix_len.into()).unwrap_or(0),
        )
    }

    fn last_v6(prefix: IpPrefix<Ipv6Addr>) -> Ipv6Addr {
        Ipv6Addr::from(
            u128::from(prefix.network)
                | u128::MAX.checked_shr(prefix.prefix_len.into()).unwrap_or(0),
        )
    }

    #[test]
    fn rejects_every_current_non_global_ipv4_registry_prefix() {
        for prefix in NON_GLOBAL_IPV4_PREFIXES {
            for address in [prefix.network, last_v4(*prefix)] {
                assert!(
                    !is_globally_reachable_aegs_destination(IpAddr::V4(address)),
                    "{address} from {}/{} must be rejected",
                    prefix.network,
                    prefix.prefix_len
                );
            }
        }
    }

    #[test]
    fn preserves_current_globally_reachable_ipv4_exceptions() {
        for prefix in GLOBAL_IPV4_EXCEPTIONS {
            assert!(is_globally_reachable_aegs_destination(IpAddr::V4(
                prefix.network
            )));
        }
        for address in [
            Ipv4Addr::new(8, 8, 8, 8),
            Ipv4Addr::new(192, 31, 196, 1),
            Ipv4Addr::new(192, 52, 193, 1),
            Ipv4Addr::new(192, 175, 48, 1),
        ] {
            assert!(is_globally_reachable_aegs_destination(IpAddr::V4(address)));
        }
    }

    #[test]
    fn rejects_every_current_non_global_ipv6_registry_prefix() {
        for prefix in NON_GLOBAL_IPV6_PREFIXES {
            for address in [prefix.network, last_v6(*prefix)] {
                if GLOBAL_IPV6_EXCEPTIONS
                    .iter()
                    .any(|exception| exception.contains(address))
                {
                    continue;
                }
                assert!(
                    !is_globally_reachable_aegs_destination(IpAddr::V6(address)),
                    "{address} from {}/{} must be rejected",
                    prefix.network,
                    prefix.prefix_len
                );
            }
        }
    }

    #[test]
    fn rejects_reviewer_reported_ipv6_special_purpose_ranges() {
        for address in ["100:0:0:1::1", "2001:2::1", "3fff::1", "5f00::1"] {
            let address = address.parse::<Ipv6Addr>().expect("IPv6 test vector");
            assert!(!is_globally_reachable_aegs_destination(IpAddr::V6(address)));
        }
    }

    #[test]
    fn preserves_current_globally_reachable_ipv6_exceptions() {
        for prefix in GLOBAL_IPV6_EXCEPTIONS {
            assert!(is_globally_reachable_aegs_destination(IpAddr::V6(
                prefix.network
            )));
        }
        for address in [
            "64:ff9b::808:808",
            "2001:1::1",
            "2001:1::2",
            "2001:1::3",
            "2001:3::1",
            "2001:4:112::1",
            "2001:20::1",
            "2001:30::1",
            "2620:4f:8000::1",
            "2606:4700:4700::1111",
        ] {
            let address = address.parse::<Ipv6Addr>().expect("IPv6 test vector");
            assert!(is_globally_reachable_aegs_destination(IpAddr::V6(address)));
        }
    }

    #[test]
    fn rejects_mapped_multicast_and_unallocated_ipv6_destinations() {
        for address in ["::ffff:8.8.8.8", "ff0e::1", "4000::1"] {
            let address = address.parse::<Ipv6Addr>().expect("IPv6 test vector");
            assert!(!is_globally_reachable_aegs_destination(IpAddr::V6(address)));
        }
    }
}
