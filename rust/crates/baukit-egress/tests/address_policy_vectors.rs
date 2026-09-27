//! Runs the shared vectors in `fixtures/egress/address-policy-v1.json`.

use std::{
    collections::VecDeque,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use baukit_egress::{
    AddressPolicy, EgressError, ResolveError, ResolveFuture, Resolver, is_public_address,
    resolve_destination, validate_destination,
};
use serde::Deserialize;
use url::Url;

const VECTORS: &str = include_str!("../../../../fixtures/egress/address-policy-v1.json");
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vectors {
    version: u32,
    address_cases: Vec<AddressCase>,
    destination_cases: Vec<DestinationCase>,
    resolution_cases: Vec<ResolutionCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PerPolicy<T> {
    public_only: T,
    allow_loopback: T,
}

impl<T> PerPolicy<T> {
    fn cases(&self) -> [(AddressPolicy, &T); 2] {
        [
            (AddressPolicy::PublicOnly, &self.public_only),
            (AddressPolicy::AllowLoopback, &self.allow_loopback),
        ]
    }
}

#[derive(Deserialize)]
struct AddressCase {
    name: String,
    address: IpAddr,
    expected: PerPolicy<bool>,
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
    answers: Vec<Vec<IpAddr>>,
    expected: Vec<Outcome>,
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

fn policy(name: &str) -> AddressPolicy {
    match name {
        "publicOnly" => AddressPolicy::PublicOnly,
        "allowLoopback" => AddressPolicy::AllowLoopback,
        other => panic!("unknown policy {other}"),
    }
}

#[test]
fn address_cases_match_the_policy() {
    for case in vectors().address_cases {
        for (policy, expected) in case.expected.cases() {
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
    }
}

#[test]
fn destination_cases_match_the_url_checks() {
    for case in vectors().destination_cases {
        let url = Url::parse(&case.url).expect("destination URL should parse");
        for (policy, expected) in case.expected.cases() {
            let outcome = validate_destination(&url, policy)
                .map_or_else(|error| error_outcome(&error), |()| Outcome::Accepted);
            assert_eq!(&outcome, expected, "{} under {policy:?}", case.name);
        }
    }
}

#[tokio::test]
async fn resolution_cases_check_every_answer_of_every_lookup() {
    for case in vectors().resolution_cases {
        let resolver = Arc::new(ScriptedResolver {
            answers: Mutex::new(case.answers.into_iter().collect()),
        });
        let policy = policy(&case.policy);
        for (lookup, expected) in case.expected.iter().enumerate() {
            let outcome =
                resolve_destination(resolver.as_ref(), "hooks.test", policy, LOOKUP_TIMEOUT)
                    .await
                    .map_or_else(
                        |error| error_outcome(&error),
                        |addresses| Outcome::Connect { addresses },
                    );
            assert_eq!(&outcome, expected, "{} lookup {lookup}", case.name);
        }
    }
}
