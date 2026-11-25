use std::{
    error::Error,
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::http::{Method, StatusCode};
use baukit_auth::{IdentityVerifier, OidcConfig, OidcVerifier, Principal, VerificationError};

use crate::support::{self, AUDIENCE, Quota};

const ALLOWANCE: u64 = 3;
const ATTEMPTS: usize = 20;
const LIFETIME: std::time::Duration = std::time::Duration::from_secs(60);

struct CountingVerifier {
    inner: OidcVerifier,
    calls: Arc<AtomicUsize>,
}

impl IdentityVerifier for CountingVerifier {
    fn verify<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, VerificationError>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        IdentityVerifier::verify(&self.inner, token)
    }
}

/// G-04: the IP limiter runs before token verification, so junk bearers cannot buy verifications.
#[tokio::test]
async fn bad_bearers_are_verified_at_most_once_per_ip_allowance() -> Result<(), Box<dyn Error>> {
    let issuer = baukit_test::MockOidcServer::start().await?;
    let calls = Arc::new(AtomicUsize::new(0));
    let verifier = CountingVerifier {
        inner: OidcVerifier::discover(OidcConfig::new(issuer.issuer(), AUDIENCE)?).await?,
        calls: calls.clone(),
    };
    let quota = Quota {
        requests: ALLOWANCE,
        burst: 0,
    };
    let composed = support::compose_with(&quota, None, issuer, Arc::new(verifier)).await?;
    let client: SocketAddr = "10.9.0.1:40000".parse()?;
    let mut limited = 0_usize;
    for attempt in 0..ATTEMPTS {
        let kid = format!("random-kid-{attempt}");
        let claims = composed.issuer.claims("attacker", AUDIENCE, LIFETIME)?;
        let token = composed.issuer.mint_with_key_id(&claims, &kid)?;
        let reply = support::send(
            &composed.api.router,
            support::request(
                Method::GET,
                "/items",
                client,
                Some(&format!("Bearer {token}")),
                Vec::new(),
            )?,
        )
        .await?;
        if reply.status == StatusCode::TOO_MANY_REQUESTS {
            limited += 1;
        }
    }
    let verified = calls.load(Ordering::SeqCst);
    assert!(
        u64::try_from(verified)? <= ALLOWANCE,
        "{verified} verifications for an IP allowance of {ALLOWANCE}"
    );
    assert_eq!(limited, ATTEMPTS - usize::try_from(ALLOWANCE)?);
    Ok(())
}
