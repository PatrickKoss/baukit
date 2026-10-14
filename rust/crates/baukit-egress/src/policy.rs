//! Which resolved addresses an outbound request may connect to.
//!
//! The tables follow the IANA IPv4 and IPv6 Special-Purpose Address
//! registries. The shared vectors in `fixtures/egress/address-policy-v1.json`
//! pin every decision.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    num::IntErrorKind,
    str::FromStr,
};

/// An IPv4 or IPv6 network wholly inside private or shared address space.
///
/// Accepted ranges are `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`,
/// `100.64.0.0/10`, and `fc00::/7`. The network address must have no host bits.
/// IPv4-mapped IPv6 networks are rejected; write IPv4 networks as IPv4.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrivateNetwork {
    network: IpAddr,
    prefix: u8,
}

/// A rejected private network or network list. Messages omit the input.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PrivateNetworkError {
    /// The input was not an IP address followed by a slash and prefix length.
    #[error("private network must use IP address/prefix notation")]
    InvalidNotation,
    /// The prefix was longer than the address family allows.
    #[error("private network prefix exceeds the address family length")]
    PrefixTooLong,
    /// The network address had host bits set.
    #[error("private network address must have no host bits set")]
    HostBitsSet,
    /// The network was not wholly inside an accepted private or shared range.
    #[error("private network must be wholly inside an allowed private or shared range")]
    NotPrivate,
    /// The network used an IPv4-mapped IPv6 address.
    #[error("private IPv4 networks must use IPv4 notation")]
    Ipv4Mapped,
    /// The network list was empty.
    #[error("private network list must not be empty")]
    EmptyList,
}

impl PrivateNetwork {
    /// Creates a network, rejecting invalid prefixes, host bits, and ranges.
    pub fn new(network: IpAddr, prefix: u8) -> Result<Self, PrivateNetworkError> {
        let host_bits = match network {
            IpAddr::V4(address) => {
                if u32::from(prefix) > u32::BITS {
                    return Err(PrivateNetworkError::PrefixTooLong);
                }
                address.to_bits() & ipv4_mask(u32::from(prefix)) != address.to_bits()
            }
            IpAddr::V6(address) => {
                if u32::from(prefix) > u128::BITS {
                    return Err(PrivateNetworkError::PrefixTooLong);
                }
                if address.to_ipv4_mapped().is_some() {
                    return Err(PrivateNetworkError::Ipv4Mapped);
                }
                address.to_bits() & ipv6_mask(u32::from(prefix)) != address.to_bits()
            }
        };
        if host_bits {
            return Err(PrivateNetworkError::HostBitsSet);
        }
        if !ALLOWLISTABLE_NETWORKS
            .iter()
            .any(|range| prefix >= range.prefix && range.contains(network))
        {
            return Err(PrivateNetworkError::NotPrivate);
        }
        Ok(Self { network, prefix })
    }

    /// Returns the network address without host bits.
    #[must_use]
    pub const fn network(&self) -> IpAddr {
        self.network
    }

    /// Returns the network prefix length.
    #[must_use]
    pub const fn prefix(&self) -> u8 {
        self.prefix
    }

    fn contains(&self, address: IpAddr) -> bool {
        match (address, self.network) {
            (IpAddr::V4(address), IpAddr::V4(network)) => {
                ipv4_in(address, network, u32::from(self.prefix))
            }
            (IpAddr::V6(address), IpAddr::V6(network)) => {
                ipv6_in(address, (network, u32::from(self.prefix)))
            }
            _ => false,
        }
    }
}

impl FromStr for PrivateNetwork {
    type Err = PrivateNetworkError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (address, prefix) = value
            .split_once('/')
            .ok_or(PrivateNetworkError::InvalidNotation)?;
        let address = address
            .parse()
            .map_err(|_| PrivateNetworkError::InvalidNotation)?;
        let prefix = prefix.parse::<u8>().map_err(|error| match error.kind() {
            IntErrorKind::PosOverflow => PrivateNetworkError::PrefixTooLong,
            _ => PrivateNetworkError::InvalidNotation,
        })?;
        Self::new(address, prefix)
    }
}

/// A non-empty list of private networks a guarded client may reach.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivateNetworks(Vec<PrivateNetwork>);

impl PrivateNetworks {
    /// Creates a list, rejecting an empty iterator.
    pub fn new(
        networks: impl IntoIterator<Item = PrivateNetwork>,
    ) -> Result<Self, PrivateNetworkError> {
        let networks: Vec<_> = networks.into_iter().collect();
        if networks.is_empty() {
            return Err(PrivateNetworkError::EmptyList);
        }
        Ok(Self(networks))
    }

    /// Returns the configured networks in their original order.
    #[must_use]
    pub fn as_slice(&self) -> &[PrivateNetwork] {
        &self.0
    }

    fn permits(&self, address: IpAddr) -> bool {
        let address = address.to_canonical();
        !CLOUD_METADATA_ADDRESSES.contains(&address)
            && self.0.iter().any(|network| network.contains(address))
    }
}

const ALLOWLISTABLE_NETWORKS: [PrivateNetwork; 5] = [
    PrivateNetwork {
        network: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0)),
        prefix: 8,
    },
    PrivateNetwork {
        network: IpAddr::V4(Ipv4Addr::new(172, 16, 0, 0)),
        prefix: 12,
    },
    PrivateNetwork {
        network: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 0)),
        prefix: 16,
    },
    PrivateNetwork {
        network: IpAddr::V4(Ipv4Addr::new(100, 64, 0, 0)),
        prefix: 10,
    },
    PrivateNetwork {
        network: IpAddr::V6(Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0)),
        prefix: 7,
    },
];

const CLOUD_METADATA_ADDRESSES: [IpAddr; 2] = [
    IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
    IpAddr::V6(Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254)),
];

/// Decides which resolved addresses a guarded client may connect to.
///
/// Every DNS answer must pass the policy. Cloud metadata addresses are always
/// denied. IPv4-mapped answers are judged and connected as plain IPv4.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum AddressPolicy {
    /// Only globally reachable unicast addresses, over HTTPS.
    #[default]
    PublicOnly,
    /// Public addresses plus loopback, and plain HTTP to loopback only.
    ///
    /// Meant for local development and tests against servers on the same
    /// host. Never enable it in a deployed environment.
    AllowLoopback,
    /// Public addresses over HTTPS, plus explicitly listed private networks.
    ///
    /// Plain HTTP reaches listed networks only. Every answer for a plain HTTP
    /// host name must be inside the list. Loopback and link-local stay blocked.
    /// NAT64 addresses carrying private IPv4 addresses do not pass the list.
    AllowPrivate(PrivateNetworks),
}

impl AddressPolicy {
    /// Returns whether a connection to `address` is allowed.
    #[must_use]
    pub fn permits(&self, address: IpAddr) -> bool {
        if CLOUD_METADATA_ADDRESSES.contains(&address.to_canonical()) {
            return false;
        }
        is_public_address(address)
            || match self {
                Self::PublicOnly => false,
                Self::AllowLoopback => address.to_canonical().is_loopback(),
                Self::AllowPrivate(networks) => networks.permits(address),
            }
    }

    /// Returns whether `answers` is non-empty and every answer is allowed.
    ///
    /// One disallowed answer rejects the whole set, wherever it appears.
    #[must_use]
    pub fn permits_all(&self, answers: &[IpAddr]) -> bool {
        !answers.is_empty() && answers.iter().all(|address| self.permits(*address))
    }

    /// Returns whether the policy accepts plain `http` for any destination.
    ///
    /// [`AddressPolicy::permits_plain_http`] limits it to loopback or listed networks.
    #[must_use]
    pub const fn allows_plain_http(&self) -> bool {
        matches!(self, Self::AllowLoopback | Self::AllowPrivate(_))
    }

    /// Returns whether a plain `http` connection to `address` is allowed.
    ///
    /// [`AddressPolicy::AllowLoopback`] permits loopback only.
    /// [`AddressPolicy::AllowPrivate`] permits listed networks only, excluding
    /// cloud metadata. Both accept IPv4-mapped forms of allowed IPv4 addresses.
    #[must_use]
    pub fn permits_plain_http(&self, address: IpAddr) -> bool {
        match self {
            Self::PublicOnly => false,
            Self::AllowLoopback => address.to_canonical().is_loopback(),
            Self::AllowPrivate(networks) => networks.permits(address),
        }
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
    let mask = ipv4_mask(prefix);
    address.to_bits() & mask == network.to_bits() & mask
}

fn ipv6_in(address: Ipv6Addr, (network, prefix): (Ipv6Addr, u32)) -> bool {
    let mask = ipv6_mask(prefix);
    address.to_bits() & mask == network.to_bits() & mask
}

fn ipv4_mask(prefix: u32) -> u32 {
    u32::MAX.checked_shl(u32::BITS - prefix).unwrap_or(0)
}

fn ipv6_mask(prefix: u32) -> u128 {
    u128::MAX.checked_shl(u128::BITS - prefix).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private_policy(networks: &[&str]) -> AddressPolicy {
        AddressPolicy::AllowPrivate(
            PrivateNetworks::new(
                networks
                    .iter()
                    .map(|network| network.parse().expect("valid network")),
            )
            .expect("non-empty networks"),
        )
    }

    #[test]
    fn private_networks_parse_and_construct_at_family_boundaries() {
        for (network, address, prefix) in [
            ("10.0.0.0/8", "10.0.0.0", 8),
            ("172.16.0.0/12", "172.16.0.0", 12),
            ("192.168.0.0/16", "192.168.0.0", 16),
            ("100.64.0.0/10", "100.64.0.0", 10),
            ("10.43.0.0/16", "10.43.0.0", 16),
            ("10.43.0.10/32", "10.43.0.10", 32),
            ("fc00::/7", "fc00::", 7),
            ("fd12:3456::/48", "fd12:3456::", 48),
            ("fd12::1/128", "fd12::1", 128),
        ] {
            let address = address.parse().expect("valid address");
            let parsed: PrivateNetwork = network.parse().expect("valid private network");
            assert_eq!(PrivateNetwork::new(address, prefix), Ok(parsed));
            assert_eq!(parsed.network(), address);
            assert_eq!(parsed.prefix(), prefix);
        }
    }

    #[test]
    fn private_networks_reject_invalid_construction() {
        for (address, prefix, expected) in [
            ("10.0.0.0", 33, PrivateNetworkError::PrefixTooLong),
            ("fd00::", 129, PrivateNetworkError::PrefixTooLong),
            ("10.0.0.1", 8, PrivateNetworkError::HostBitsSet),
            ("fd12:3456::1", 48, PrivateNetworkError::HostBitsSet),
            ("::ffff:10.0.0.0", 104, PrivateNetworkError::Ipv4Mapped),
        ] {
            assert_eq!(
                PrivateNetwork::new(address.parse().expect("valid address"), prefix),
                Err(expected)
            );
        }
        for network in [
            "0.0.0.0/0",
            "::/0",
            "10.0.0.0/7",
            "172.0.0.0/11",
            "192.168.0.0/15",
            "100.0.0.0/9",
            "fc00::/6",
            "8.8.8.0/24",
            "127.0.0.0/8",
            "::1/128",
            "169.254.0.0/16",
            "fe80::/10",
            "224.0.0.0/4",
            "ff00::/8",
            "192.0.2.0/24",
            "2001:db8::/32",
        ] {
            assert_eq!(
                network.parse::<PrivateNetwork>(),
                Err(PrivateNetworkError::NotPrivate),
                "{network}"
            );
        }
    }

    #[test]
    fn private_networks_reject_invalid_notation_without_echoing_it() {
        for network in [
            "url-secret",
            "10.0.0.0",
            "10.0.0.0/",
            "10.0.0.0/-1",
            "10.0.0.0/8/9",
            "bad-address/8",
        ] {
            let error = network
                .parse::<PrivateNetwork>()
                .expect_err("invalid notation");
            assert_eq!(error, PrivateNetworkError::InvalidNotation);
            assert!(!error.to_string().contains(network));
        }
        assert_eq!(
            "10.0.0.0/256".parse::<PrivateNetwork>(),
            Err(PrivateNetworkError::PrefixTooLong)
        );
    }

    #[test]
    fn private_network_list_must_be_non_empty() {
        assert_eq!(
            PrivateNetworks::new([]),
            Err(PrivateNetworkError::EmptyList)
        );
        let network = "10.43.0.0/16".parse().expect("valid network");
        let networks = PrivateNetworks::new([network]).expect("non-empty networks");
        assert_eq!(networks.as_slice(), &[network]);
    }

    #[test]
    fn private_policy_allows_only_public_and_listed_addresses() {
        let policy = private_policy(&["10.43.0.0/16", "fc00::/7"]);
        for address in [
            "10.43.0.0",
            "10.43.0.10",
            "10.43.255.255",
            "::ffff:10.43.0.10",
            "fd12::1",
            "8.8.8.8",
            "2606:4700:4700::1111",
            "64:ff9b::808:808",
        ] {
            assert!(
                policy.permits(address.parse().expect("valid address")),
                "{address}"
            );
        }
        for address in [
            "10.42.255.255",
            "10.44.0.1",
            "169.254.169.254",
            "::ffff:169.254.169.254",
            "fd00:ec2::254",
            "127.0.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "fe80::1",
            "192.0.2.1",
            "224.0.0.1",
            "ff02::1",
            "64:ff9b::a2b:a",
        ] {
            assert!(
                !policy.permits(address.parse().expect("valid address")),
                "{address}"
            );
        }
        assert!(!policy.permits_all(&[]));
        let listed = "10.43.0.10".parse().expect("valid address");
        let unlisted = "10.44.0.1".parse().expect("valid address");
        assert!(!policy.permits_all(&[listed, unlisted]));
        assert!(!policy.permits_all(&[unlisted, listed]));
    }

    #[test]
    fn private_policy_allows_plain_http_only_to_listed_addresses() {
        let policy = private_policy(&["10.43.0.0/16", "fc00::/7"]);
        assert!(policy.allows_plain_http());
        for address in ["10.43.0.10", "::ffff:10.43.0.10", "fd12::1"] {
            assert!(
                policy.permits_plain_http(address.parse().expect("valid address")),
                "{address}"
            );
        }
        for address in [
            "8.8.8.8",
            "10.44.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "fd00:ec2::254",
            "64:ff9b::a2b:a",
        ] {
            assert!(
                !policy.permits_plain_http(address.parse().expect("valid address")),
                "{address}"
            );
        }
    }
}
