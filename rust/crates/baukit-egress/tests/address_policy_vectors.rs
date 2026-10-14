//! Runs the shared vectors in `fixtures/egress/address-policy-v1.json`.

use std::{
    collections::VecDeque,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use baukit_egress::{
    AddressPolicy, DestinationRejection, EgressError, PrivateNetwork, PrivateNetworkError,
    PrivateNetworks, ResolveError, ResolveFuture, Resolver, is_public_address, resolve_destination,
    validate_destination,
};
use serde::Deserialize;
use url::Url;

const VECTORS: &str = include_str!("../../../../fixtures/egress/address-policy-v1.json");
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vectors {
    version: u32,
    private_networks: Vec<String>,
    address_cases: Vec<AddressCase>,
    destination_cases: Vec<DestinationCase>,
    resolution_cases: Vec<ResolutionCase>,
    network_cases: Vec<NetworkCase>,
    network_list_cases: Vec<NetworkListCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PerPolicy<T> {
    public_only: T,
    allow_loopback: T,
    allow_private: T,
}

impl<T> PerPolicy<T> {
    fn cases(&self, networks: &[String]) -> [(AddressPolicy, &T); 3] {
        [
            (AddressPolicy::PublicOnly, &self.public_only),
            (AddressPolicy::AllowLoopback, &self.allow_loopback),
            (policy("allowPrivate", networks), &self.allow_private),
        ]
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddressCase {
    name: String,
    address: IpAddr,
    expected: PerPolicy<bool>,
    plain_http_expected: Option<PerPolicy<bool>>,
}

#[derive(Deserialize)]
struct DestinationCase {
    name: String,
    url: String,
    expected: PerPolicy<Outcome>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolutionCase {
    name: String,
    policy: String,
    private_networks: Option<Vec<String>>,
    #[serde(default)]
    plain_http: bool,
    answers: Vec<Vec<IpAddr>>,
    expected: Vec<Outcome>,
}

#[derive(Deserialize)]
struct NetworkCase {
    network: String,
    expected: NetworkOutcome,
}

#[derive(Deserialize)]
struct NetworkListCase {
    networks: Vec<String>,
    expected: NetworkOutcome,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum NetworkOutcome {
    Accepted,
    InvalidNotation,
    PrefixTooLong,
    HostBitsSet,
    NotPrivate,
    Ipv4Mapped,
    EmptyList,
}

fn network_outcome<T>(result: Result<T, PrivateNetworkError>) -> NetworkOutcome {
    match result {
        Ok(_) => NetworkOutcome::Accepted,
        Err(PrivateNetworkError::InvalidNotation) => NetworkOutcome::InvalidNotation,
        Err(PrivateNetworkError::PrefixTooLong) => NetworkOutcome::PrefixTooLong,
        Err(PrivateNetworkError::HostBitsSet) => NetworkOutcome::HostBitsSet,
        Err(PrivateNetworkError::NotPrivate) => NetworkOutcome::NotPrivate,
        Err(PrivateNetworkError::Ipv4Mapped) => NetworkOutcome::Ipv4Mapped,
        Err(PrivateNetworkError::EmptyList) => NetworkOutcome::EmptyList,
    }
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum Outcome {
    Accepted,
    Connect { addresses: Vec<IpAddr> },
    BlockedAddress,
    DestinationNotAllowed { reason: String },
    ResolveFailed,
}

fn vectors() -> Vectors {
    let vectors: Vectors = serde_json::from_str(VECTORS).expect("vectors should parse");
    assert_eq!(vectors.version, 1);
    vectors
}

fn error_outcome(error: &EgressError) -> Outcome {
    match error {
        EgressError::BlockedAddress => Outcome::BlockedAddress,
        EgressError::Resolve => Outcome::ResolveFailed,
        EgressError::Destination(reason) => Outcome::DestinationNotAllowed {
            reason: reason.code().to_owned(),
        },
        other => panic!("unexpected error {other}"),
    }
}

struct ScriptedResolver {
    answers: Mutex<VecDeque<Vec<IpAddr>>>,
}

impl Resolver for ScriptedResolver {
    fn resolve<'a>(&'a self, _host: &'a str) -> ResolveFuture<'a> {
        let answer = self
            .answers
            .lock()
            .expect("answers lock should not be poisoned")
            .pop_front()
            .ok_or_else(ResolveError::not_found);
        Box::pin(async move { answer })
    }
}

fn policy(name: &str, networks: &[String]) -> AddressPolicy {
    match name {
        "publicOnly" => AddressPolicy::PublicOnly,
        "allowLoopback" => AddressPolicy::AllowLoopback,
        "allowPrivate" => AddressPolicy::AllowPrivate(
            PrivateNetworks::new(
                networks
                    .iter()
                    .map(|network| network.parse().expect("valid network")),
            )
            .expect("non-empty networks"),
        ),
        other => panic!("unknown policy {other}"),
    }
}

#[test]
fn address_cases_match_the_policy() {
    let vectors = vectors();
    for case in vectors.address_cases {
        for (policy, expected) in case.expected.cases(&vectors.private_networks) {
            assert_eq!(
                policy.permits(case.address),
                *expected,
                "{} under {policy:?}",
                case.name
            );
        }
        assert_eq!(
            is_public_address(case.address),
            case.expected.public_only,
            "{}",
            case.name
        );
        if let Some(expected) = case.plain_http_expected {
            for (policy, expected) in expected.cases(&vectors.private_networks) {
                assert_eq!(
                    policy.permits_plain_http(case.address),
                    *expected,
                    "{} plain HTTP under {policy:?}",
                    case.name
                );
            }
        }
    }
}

#[test]
fn destination_cases_match_the_url_checks() {
    let vectors = vectors();
    for case in vectors.destination_cases {
        let url = Url::parse(&case.url).expect("destination URL should parse");
        for (policy, expected) in case.expected.cases(&vectors.private_networks) {
            let outcome = validate_destination(&url, &policy)
                .map_or_else(|error| error_outcome(&error), |()| Outcome::Accepted);
            assert_eq!(&outcome, expected, "{} under {policy:?}", case.name);
        }
    }
}

#[tokio::test]
async fn resolution_cases_check_every_answer_of_every_lookup() {
    let vectors = vectors();
    for case in vectors.resolution_cases {
        assert_eq!(
            case.answers.len(),
            case.expected.len(),
            "{} lookup count",
            case.name
        );
        let resolver = Arc::new(ScriptedResolver {
            answers: Mutex::new(case.answers.into_iter().collect()),
        });
        let policy = policy(
            &case.policy,
            case.private_networks
                .as_deref()
                .unwrap_or(&vectors.private_networks),
        );
        for (lookup, expected) in case.expected.iter().enumerate() {
            let outcome =
                resolve_destination(resolver.as_ref(), "hooks.test", &policy, LOOKUP_TIMEOUT)
                    .await
                    .map_or_else(
                        |error| error_outcome(&error),
                        |addresses| {
                            if case.plain_http
                                && !addresses
                                    .iter()
                                    .all(|address| policy.permits_plain_http(*address))
                            {
                                error_outcome(&EgressError::Destination(
                                    DestinationRejection::Scheme,
                                ))
                            } else {
                                Outcome::Connect { addresses }
                            }
                        },
                    );
            assert_eq!(&outcome, expected, "{} lookup {lookup}", case.name);
        }
    }
}

#[test]
fn network_construction_cases_match_typed_errors() {
    for case in vectors().network_cases {
        let parsed = case.network.parse::<PrivateNetwork>();
        assert_eq!(network_outcome(parsed), case.expected, "{}", case.network);
        if let Some((address, prefix)) = case.network.split_once('/')
            && let (Ok(address), Ok(prefix)) = (address.parse(), prefix.parse())
        {
            assert_eq!(
                PrivateNetwork::new(address, prefix),
                parsed,
                "{} constructor",
                case.network
            );
        }
    }
}

#[test]
fn network_list_cases_require_a_non_empty_list() {
    for case in vectors().network_list_cases {
        let networks = case
            .networks
            .iter()
            .map(|network| network.parse().expect("valid network"));
        assert_eq!(
            network_outcome(PrivateNetworks::new(networks)),
            case.expected,
            "{:?}",
            case.networks
        );
    }
}
