#![no_main]

use std::{net::IpAddr, sync::LazyLock};

use baukit_egress::{AddressPolicy, AllowedEndpoint, PrivateNetworks, validate_destination};
use libfuzzer_sys::fuzz_target;
use url::{Host, Url};

static POLICIES: LazyLock<Vec<AddressPolicy>> = LazyLock::new(|| {
    let networks = PrivateNetworks::new(
        ["10.43.0.0/16", "fc00::/7"].map(|network| network.parse().expect("valid network")),
    )
    .expect("non-empty networks");
    vec![
        AddressPolicy::PublicOnly,
        AddressPolicy::PublicHttp,
        AddressPolicy::AllowLoopback,
        AddressPolicy::AllowPrivate(networks),
        AddressPolicy::OnlyEndpoint(
            AllowedEndpoint::new("127.0.0.1", 8080, true).expect("internal endpoint"),
        ),
        AddressPolicy::OnlyEndpoint(
            AllowedEndpoint::tenant("8.8.8.8", 443, false).expect("tenant endpoint"),
        ),
    ]
});

fn literal(url: &Url) -> Option<IpAddr> {
    match url.host()? {
        Host::Ipv4(address) => Some(IpAddr::V4(address)),
        Host::Ipv6(address) => Some(IpAddr::V6(address)),
        Host::Domain(_) => None,
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(url) = Url::parse(text) else {
        return;
    };
    let address = literal(&url);
    for policy in POLICIES.iter() {
        if validate_destination(&url, policy).is_err() {
            continue;
        }
        let Some(address) = address else {
            continue;
        };
        assert!(policy.permits(address), "{policy:?} accepted {address}");
        if url.scheme() == "http" {
            assert!(
                policy.permits_plain_http(address),
                "{policy:?} http {address}"
            );
        }
    }
});
