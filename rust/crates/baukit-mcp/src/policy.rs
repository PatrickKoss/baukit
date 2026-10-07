use std::{collections::BTreeSet, future::Future, pin::Pin, time::Duration};

pub use baukit_auth::Principal as VerifiedPrincipal;

/// Identity and grants after a product's authentication policy has run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Principal {
    identity: VerifiedPrincipal,
    subject: String,
    scopes: BTreeSet<String>,
}

impl From<VerifiedPrincipal> for Principal {
    fn from(identity: VerifiedPrincipal) -> Self {
        Self {
            subject: identity.subject().to_owned(),
            scopes: identity.scopes().clone(),
            identity,
        }
    }
}

impl Principal {
    /// Creates an internal identity for service tests, without OAuth grants.
    pub fn new(subject: impl Into<String>) -> Self {
        VerifiedPrincipal::new(subject).into()
    }

    /// Maps the verified identity to a product account after a trusted lookup.
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = subject.into();
        self
    }

    /// Restricts effective grants. Policies cannot add scopes absent from the JWT.
    pub fn with_scopes(mut self, scopes: impl IntoIterator<Item = String>) -> Self {
        self.scopes = scopes
            .into_iter()
            .filter(|scope| self.scopes.contains(scope))
            .collect();
        self
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn issuer(&self) -> Option<&str> {
        self.identity.issuer()
    }

    pub fn client_id(&self) -> Option<&str> {
        self.identity.client_id()
    }

    pub fn scopes(&self) -> &BTreeSet<String> {
        &self.scopes
    }

    /// Original verified issuer, subject, client and claims for account policies.
    pub fn verified_identity(&self) -> &VerifiedPrincipal {
        &self.identity
    }
}

/// A safe denial. Provider and database details must stay inside the adapter.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PolicyDenial {
    #[error("inactive or revoked identity")]
    Inactive,
    #[error("required scopes are missing")]
    InsufficientScope(Vec<String>),
    #[error("authentication policy unavailable")]
    Unavailable,
    #[error("product quota exhausted")]
    RateLimited(Duration),
}

pub type PolicyFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Principal, PolicyDenial>> + Send + 'a>>;

/// Runs for every authenticated HTTP request, after signature, issuer, audience and expiry checks.
/// Account lookups and erasure fences belong here, before tool discovery or execution.
pub trait AuthenticationPolicy: Send + Sync + 'static {
    fn authenticate<'a>(
        &'a self,
        principal: &'a VerifiedPrincipal,
        token: &'a str,
    ) -> PolicyFuture<'a>;
}

/// Uses the verified JWT identity and grants until the token expires.
pub struct JwtOnlyPolicy;

impl AuthenticationPolicy for JwtOnlyPolicy {
    fn authenticate<'a>(
        &'a self,
        principal: &'a VerifiedPrincipal,
        _token: &'a str,
    ) -> PolicyFuture<'a> {
        Box::pin(async { Ok(principal.clone().into()) })
    }
}
