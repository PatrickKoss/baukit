//! The DNS resolution port and its adapters.

use std::{collections::HashMap, future::Future, net::IpAddr, pin::Pin, time::Duration};

use crate::{AddressPolicy, EgressError};

/// A boxed future returned by [`Resolver::resolve`].
pub type ResolveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<IpAddr>, ResolveError>> + Send + 'a>>;

/// Resolves a host name to every address it currently answers with.
///
/// Implementations return all answers. The guarded client rejects the whole
/// set when any answer is not allowed, so filtering here would hide an unsafe
/// answer instead of rejecting it.
pub trait Resolver: Send + Sync {
    /// Resolves `host`, a name without port or brackets.
    fn resolve<'a>(&'a self, host: &'a str) -> ResolveFuture<'a>;
}

/// A resolver lookup that failed or found no such name.
#[derive(Debug, thiserror::Error)]
#[error("DNS lookup failed")]
pub struct ResolveError {
    #[source]
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl ResolveError {
    /// Wraps the resolver's own error.
    pub fn new(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self {
            source: Some(source.into()),
        }
    }

    /// Reports a name the resolver does not know.
    #[must_use]
    pub const fn not_found() -> Self {
        Self { source: None }
    }
}

/// Resolves through the operating system, as `getaddrinfo` does.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemResolver;

impl Resolver for SystemResolver {
    fn resolve<'a>(&'a self, host: &'a str) -> ResolveFuture<'a> {
        Box::pin(async move {
            let answers = tokio::net::lookup_host((host, 0))
                .await
                .map_err(ResolveError::new)?;
            Ok(answers.map(|address| address.ip()).collect())
        })
    }
}

/// Answers from a fixed table of host names.
///
/// Useful in tests that point a real host name at a local server, and for
/// hosts whose addresses are configured rather than looked up. Names match
/// without regard to ASCII case.
#[derive(Clone, Debug, Default)]
pub struct StaticResolver {
    hosts: HashMap<String, Vec<IpAddr>>,
}

impl StaticResolver {
    /// Creates a resolver that knows no names.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers `host` with `addresses`, replacing any earlier entry.
    #[must_use]
    pub fn with_host(
        mut self,
        host: impl Into<String>,
        addresses: impl IntoIterator<Item = IpAddr>,
    ) -> Self {
        self.hosts.insert(
            host.into().to_ascii_lowercase(),
            addresses.into_iter().collect(),
        );
        self
    }
}

impl Resolver for StaticResolver {
    fn resolve<'a>(&'a self, host: &'a str) -> ResolveFuture<'a> {
        let answers = self
            .hosts
            .get(&host.to_ascii_lowercase())
            .cloned()
            .ok_or_else(ResolveError::not_found);
        Box::pin(async move { answers })
    }
}

/// Resolves `host` and returns the answers a connection may use.
///
/// Every answer must pass `policy`; a single disallowed answer rejects the
/// whole lookup with [`EgressError::BlockedAddress`]. IPv4-mapped answers
/// come back as plain IPv4 addresses. An empty answer or a failed lookup is
/// [`EgressError::Resolve`], and a lookup slower than `timeout` is
/// [`EgressError::Timeout`].
///
/// The guarded client runs this once per new connection and connects only to
/// the returned addresses. Call it directly to check a destination early, for
/// example when a subscription is created.
pub async fn resolve_destination(
    resolver: &dyn Resolver,
    host: &str,
    policy: AddressPolicy,
    timeout: Duration,
) -> Result<Vec<IpAddr>, EgressError> {
    let answers = tokio::time::timeout(timeout, resolver.resolve(host))
        .await
        .map_err(|_| EgressError::Timeout)?
        .map_err(|error| {
            tracing::debug!(target: "baukit_egress", %error, host, "egress DNS lookup failed");
            EgressError::Resolve
        })?;
    if answers.is_empty() {
        return Err(EgressError::Resolve);
    }
    if !policy.permits_all(&answers) {
        return Err(EgressError::BlockedAddress);
    }
    Ok(answers
        .into_iter()
        .map(|address| address.to_canonical())
        .collect())
}
