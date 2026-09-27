//! Which resolved addresses an outbound request may connect to.
//!
//! The tables follow the IANA IPv4 and IPv6 Special-Purpose Address
//! registries. The shared vectors in `fixtures/egress/address-policy-v1.json`
//! pin every decision.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Decides which resolved addresses a guarded client may connect to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AddressPolicy {
    /// Only globally reachable unicast addresses, over HTTPS.
    #[default]
    PublicOnly,
    /// Public addresses plus loopback, and plain HTTP to either.
    ///
    /// Meant for local development and tests against servers on the same
    /// host. Never enable it in a deployed environment.
    AllowLoopback,
}

impl AddressPolicy {
    /// Returns whether a connection to `address` is allowed.
    #[must_use]
    pub fn permits(self, address: IpAddr) -> bool {
        is_public_address(address)
            || (self == Self::AllowLoopback && address.to_canonical().is_loopback())
    }

    /// Returns whether `answers` is non-empty and every answer is allowed.
    ///
    /// One disallowed answer rejects the whole set, wherever it appears.
    #[must_use]
    pub fn permits_all(self, answers: &[IpAddr]) -> bool {
        !answers.is_empty() && answers.iter().all(|address| self.permits(*address))
    }

    /// Returns whether plain `http` destinations are accepted.
    #[must_use]
    pub const fn allows_plain_http(self) -> bool {
        matches!(self, Self::AllowLoopback)
    }
}

const NON_PUBLIC_IPV4: [(Ipv4Addr, u32); 15] = [
    (Ipv4Addr::new(0, 0, 0, 0), 8),
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(100, 64, 0, 0), 10),
    (Ipv4Addr::new(127, 0, 0, 0), 8),
    (Ipv4Addr::new(169, 254, 0, 0), 16),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 0, 0, 0), 24),
    (Ipv4Addr::new(192, 0, 2, 0), 24),
    (Ipv4Addr::new(192, 88, 99, 0), 24),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    (Ipv4Addr::new(198, 18, 0, 0), 15),
    (Ipv4Addr::new(198, 51, 100, 0), 24),
    (Ipv4Addr::new(203, 0, 113, 0), 24),
    (Ipv4Addr::new(224, 0, 0, 0), 4),
    (Ipv4Addr::new(240, 0, 0, 0), 4),
];

const GLOBAL_UNICAST: (Ipv6Addr, u32) = (Ipv6Addr::new(0x2000, 0, 0, 0, 0, 0, 0, 0), 3);

const NAT64_WELL_KNOWN: (Ipv6Addr, u32) = (Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, 0, 0), 96);

const NON_PUBLIC_GLOBAL_UNICAST: [(Ipv6Addr, u32); 4] = [
    (Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 23),
    (Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0), 32),
    (Ipv6Addr::new(0x2002, 0, 0, 0, 0, 0, 0, 0), 16),
    (Ipv6Addr::new(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20),
];

/// Returns whether `address` is a globally reachable unicast address.
///
/// IPv4-mapped IPv6 addresses and addresses in the NAT64 well-known prefix
/// `64:ff9b::/96` are judged by the IPv4 address they carry.
#[must_use]
pub fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    !NON_PUBLIC_IPV4
        .iter()
        .any(|&(network, prefix)| ipv4_in(address, network, prefix))
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    if ipv6_in(address, NAT64_WELL_KNOWN) {
        return is_public_ipv4(embedded_ipv4(address));
    }
    ipv6_in(address, GLOBAL_UNICAST)
        && !NON_PUBLIC_GLOBAL_UNICAST
            .iter()
            .any(|&block| ipv6_in(address, block))
}

fn embedded_ipv4(address: Ipv6Addr) -> Ipv4Addr {
    let [.., a, b, c, d] = address.octets();
    Ipv4Addr::new(a, b, c, d)
}

fn ipv4_in(address: Ipv4Addr, network: Ipv4Addr, prefix: u32) -> bool {
    let mask = u32::MAX.checked_shl(u32::BITS - prefix).unwrap_or(0);
    address.to_bits() & mask == network.to_bits() & mask
}

fn ipv6_in(address: Ipv6Addr, (network, prefix): (Ipv6Addr, u32)) -> bool {
    let mask = u128::MAX.checked_shl(u128::BITS - prefix).unwrap_or(0);
    address.to_bits() & mask == network.to_bits() & mask
}
