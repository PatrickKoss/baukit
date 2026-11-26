//! Client-IP selection from `X-Forwarded-For` and IP bucket derivation.

use std::net::{IpAddr, Ipv6Addr};

const IPV6_BITS: u8 = 128;

/// Selects the client IP from an `X-Forwarded-For` entry list.
///
/// `hops` counts trusted proxies including the socket peer. Zero, a missing peer,
/// or a list shorter than `hops` yields the peer. Otherwise only the entry at index
/// `len - hops` is parsed; entries to its left are client-controlled and never read.
/// An unparseable trusted entry logs a warning and yields the peer.
#[must_use]
pub fn select_forwarded_ip(entries: &[&str], peer: Option<IpAddr>, hops: usize) -> Option<IpAddr> {
    select_forwarded_with(entries, peer, hops, |entry| {
        let parsed = entry.trim().parse().ok();
        if parsed.is_none() {
            tracing::warn!("trusted X-Forwarded-For entry is not an IP address; using the peer");
        }
        parsed
    })
}

/// [`select_forwarded_ip`] with the entry parser supplied by the caller.
pub(crate) fn select_forwarded_with<T>(
    entries: &[T],
    peer: Option<IpAddr>,
    hops: usize,
    parse: impl Fn(&T) -> Option<IpAddr>,
) -> Option<IpAddr> {
    if hops == 0 {
        return peer;
    }
    peer?;
    entries
        .len()
        .checked_sub(hops)
        .and_then(|index| entries.get(index))
        .and_then(parse)
        .or(peer)
}

/// Maps a client address to the address that names its rate-limit bucket.
///
/// IPv4 and IPv4-mapped IPv6 addresses bucket per IPv4 address. Other IPv6
/// addresses keep their first `v6_prefix` bits (clamped to 128).
#[must_use]
pub fn ip_bucket(ip: IpAddr, v6_prefix: u8) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from_bits(v6.to_bits() & ipv6_mask(v6_prefix))),
    }
}

fn ipv6_mask(prefix: u8) -> u128 {
    let prefix = prefix.min(IPV6_BITS);
    u128::MAX
        .checked_shl(u32::from(IPV6_BITS - prefix))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("IP")
    }

    #[test]
    fn selects_entry_at_len_minus_hops() {
        let entries = ["198.51.100.7", "10.0.0.4"];
        let peer = Some(ip("10.0.0.5"));
        assert_eq!(select_forwarded_ip(&entries, peer, 0), peer);
        assert_eq!(select_forwarded_ip(&entries, peer, 1), Some(ip("10.0.0.4")));
        assert_eq!(
            select_forwarded_ip(&entries, peer, 2),
            Some(ip("198.51.100.7"))
        );
        assert_eq!(select_forwarded_ip(&entries, peer, 3), peer);
        assert_eq!(select_forwarded_ip(&entries, None, 1), None);
    }

    #[test]
    fn client_written_garbage_left_of_the_trusted_entry_is_ignored() {
        let peer = Some(ip("10.0.0.5"));
        let entries = ["not-an-ip", "garbage, more", "203.0.113.9"];
        assert_eq!(
            select_forwarded_ip(&entries, peer, 1),
            Some(ip("203.0.113.9"))
        );
    }

    #[test]
    fn unparseable_trusted_entry_falls_back_to_the_peer() {
        let peer = Some(ip("10.0.0.5"));
        assert_eq!(select_forwarded_ip(&["203.0.113.9", "x"], peer, 1), peer);
    }

    #[test]
    fn ipv6_buckets_by_prefix_and_mapped_ipv4_as_ipv4() {
        assert_eq!(
            ip_bucket(ip("2001:db8:1:2:3:4:5:6"), 64),
            ip("2001:db8:1:2::")
        );
        assert_eq!(ip_bucket(ip("2001:db8::1"), 128), ip("2001:db8::1"));
        assert_eq!(ip_bucket(ip("2001:db8::1"), 0), ip("::"));
        assert_eq!(ip_bucket(ip("::ffff:192.0.2.1"), 64), ip("192.0.2.1"));
        assert_eq!(ip_bucket(ip("192.0.2.1"), 64), ip("192.0.2.1"));
    }
}

#[cfg(kani)]
mod proofs {
    use std::net::Ipv4Addr;

    use super::*;

    const MAX_ENTRIES: usize = 6;
    const MAX_HOPS: usize = 4;

    fn any_ip() -> Option<IpAddr> {
        if kani::any() {
            Some(IpAddr::V4(Ipv4Addr::from_bits(kani::any())))
        } else {
            None
        }
    }

    fn any_entries() -> ([Option<IpAddr>; MAX_ENTRIES], usize) {
        let entries = [any_ip(), any_ip(), any_ip(), any_ip(), any_ip(), any_ip()];
        let len: usize = kani::any();
        kani::assume(len <= MAX_ENTRIES);
        (entries, len)
    }

    fn select(entries: &[Option<IpAddr>], peer: Option<IpAddr>, hops: usize) -> Option<IpAddr> {
        select_forwarded_with(entries, peer, hops, |entry| *entry)
    }

    /// K-01: two XFF lists that agree on their rightmost `hops` entries give equal results.
    ///
    /// Entries are modelled as already-parsed `Option<IpAddr>` (IPv4 only, `None` for an
    /// unparseable string) because CBMC cannot handle `str::parse` symbolically; the
    /// parser is the only part of [`select_forwarded_ip`] this abstracts. Lists hold at
    /// most 6 entries and hops ranges over 0..=4. Unwind 8 covers the suffix comparison.
    #[kani::proof]
    #[kani::unwind(8)]
    fn forwarded_selection_ignores_client_controlled_entries() {
        let (left, left_len) = any_entries();
        let (right, right_len) = any_entries();
        let hops: usize = kani::any();
        kani::assume(hops <= MAX_HOPS && hops <= left_len && hops <= right_len);
        for offset in 1..=hops {
            kani::assume(left[left_len - offset] == right[right_len - offset]);
        }
        let peer = any_ip();
        assert_eq!(
            select(&left[..left_len], peer, hops),
            select(&right[..right_len], peer, hops)
        );
    }

    /// K-02: the result is the peer or the entry at `len - hops`, never anything else.
    /// Same entry model and bounds as K-01.
    #[kani::proof]
    #[kani::unwind(8)]
    fn forwarded_selection_is_peer_or_trusted_entry() {
        let (entries, len) = any_entries();
        let hops: usize = kani::any();
        kani::assume(hops <= MAX_HOPS);
        let peer = any_ip();
        let result = select(&entries[..len], peer, hops);
        let trusted = len
            .checked_sub(hops)
            .filter(|_| hops > 0)
            .map(|index| entries[index]);
        assert!(result == peer || trusted.is_some_and(|entry| entry == result));
    }

    /// K-19: two IPv6 addresses share a bucket iff they share the first `prefix` bits;
    /// IPv4-mapped addresses bucket as their IPv4 address. Symbolic 128-bit addresses
    /// and prefixes 0..=128; no loops.
    #[kani::proof]
    fn ip_bucket_groups_exactly_by_prefix() {
        let prefix: u8 = kani::any();
        kani::assume(prefix <= IPV6_BITS);
        let left = Ipv6Addr::from_bits(kani::any());
        let right = Ipv6Addr::from_bits(kani::any());
        kani::assume(left.to_ipv4_mapped().is_none() && right.to_ipv4_mapped().is_none());
        let shift = u32::from(IPV6_BITS - prefix);
        let same_prefix = shift == 128 || left.to_bits() >> shift == right.to_bits() >> shift;
        let same_bucket =
            ip_bucket(IpAddr::V6(left), prefix) == ip_bucket(IpAddr::V6(right), prefix);
        assert_eq!(same_prefix, same_bucket);

        let v4 = Ipv4Addr::from_bits(kani::any());
        assert_eq!(
            ip_bucket(IpAddr::V6(v4.to_ipv6_mapped()), prefix),
            IpAddr::V4(v4)
        );
    }
}
