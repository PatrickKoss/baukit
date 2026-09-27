use std::{collections::BTreeSet, error::Error, time::Duration};

use baukit_auth::{
    ClerkVerifier, IssuerVerifier, MultiIssuerError, MultiIssuerVerifier, OidcConfig, OidcVerifier,
    PrincipalClaimMapping, ProfileClaim, VerificationError, WorkOsVerifier,
};
use baukit_test::{JwtClaims, MockOidcServer};
use serde_json::json;

const LIFETIME: Duration = Duration::from_secs(300);

fn session_claims(server: &MockOidcServer, subject: &str) -> Result<JwtClaims, Box<dyn Error>> {
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .saturating_add(LIFETIME)
        .as_secs();
    Ok(JwtClaims::new()
        .subject(subject)
        .issuer(server.issuer())
        .expires_at(expires_at))
}

fn scopes<const N: usize>(values: [&str; N]) -> BTreeSet<String> {
    values.into_iter().map(str::to_owned).collect()
}

#[tokio::test]
async fn oidc_principals_carry_verified_scopes_and_selected_profile_claims()
-> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = OidcVerifier::from_jwks_uri(
        OidcConfig::new(server.issuer(), "api")?.with_principal_claims(
            PrincipalClaimMapping::new().profile_claims(["email", "email_verified", "name"]),
        ),
        server.jwks_url(),
    )?;
    let claims = server
        .claims("subject", "api", LIFETIME)?
        .claim("scope", "records:read records:write")
        .claim("email", "ada@example.com")
        .claim("email_verified", true)
        .claim("name", 42)
        .claim("phone_number", "+100");
    let principal = verifier.verify(&server.mint(&claims)?).await?;

    assert_eq!(
        principal.scopes(),
        &scopes(["records:read", "records:write"])
    );
    assert_eq!(principal.grants(), None);
    assert_eq!(
        principal
            .profile_claim("email")
            .and_then(ProfileClaim::as_str),
        Some("ada@example.com")
    );
    assert_eq!(
        principal
            .profile_claim("email_verified")
            .and_then(ProfileClaim::as_bool),
        Some(true)
    );
    assert_eq!(principal.profile_claims().len(), 2);

    let malformed = claims.claim("scope", json!({"records": "read"}));
    assert!(matches!(
        verifier.verify(&server.mint(&malformed)?).await,
        Err(VerificationError::InvalidPrincipalContext)
    ));
    Ok(())
}

#[tokio::test]
async fn a_configured_scope_claim_accepts_a_string_array() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = OidcVerifier::from_jwks_uri(
        OidcConfig::new(server.issuer(), "api")?
            .with_principal_claims(PrincipalClaimMapping::new().scope_claim("scp")),
        server.jwks_url(),
    )?;
    let claims = server
        .claims("subject", "api", LIFETIME)?
        .claim("scope", "ignored")
        .claim("scp", json!(["records:read"]));

    let principal = verifier.verify(&server.mint(&claims)?).await?;
    assert_eq!(principal.scopes(), &scopes(["records:read"]));
    assert!(principal.profile_claims().is_empty());
    Ok(())
}

#[tokio::test]
async fn clerk_checks_an_audience_only_when_one_is_configured() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let default = ClerkVerifier::from_jwks_uri(
        server.issuer(),
        ["https://app.example.com"],
        server.jwks_url(),
    )?;
    let claims = session_claims(&server, "user_clerk")?
        .claim("email", "clerk@example.com")
        .claim("scope", "records:read");
    let principal = default.verify(&server.mint(&claims)?).await?;
    assert_eq!(principal.scopes(), &scopes(["records:read"]));
    assert!(principal.profile_claims().is_empty());

    let strict = default
        .with_audiences(["orders-api"])?
        .with_profile_claims(["email"]);
    assert!(matches!(
        strict.verify(&server.mint(&claims)?).await,
        Err(VerificationError::WrongAudience)
    ));
    assert!(matches!(
        strict
            .verify(&server.mint(&claims.clone().audience("other-api"))?)
            .await,
        Err(VerificationError::WrongAudience)
    ));
    let principal = strict
        .verify(&server.mint(&claims.audience("orders-api"))?)
        .await?;
    assert_eq!(
        principal.profile_claim("email"),
        Some(&ProfileClaim::String("clerk@example.com".to_owned()))
    );
    Ok(())
}

#[tokio::test]
async fn workos_checks_an_audience_only_when_one_is_configured() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let default =
        WorkOsVerifier::from_jwks_uri(server.issuer(), "client_01ABC", server.jwks_url())?;
    let claims = session_claims(&server, "user_workos")?
        .claim("client_id", "client_01ABC")
        .claim("email_verified", false);
    default.verify(&server.mint(&claims)?).await?;

    let strict = default
        .with_audiences(["orders-api"])?
        .with_profile_claims(["email_verified"]);
    assert!(matches!(
        strict.verify(&server.mint(&claims)?).await,
        Err(VerificationError::WrongAudience)
    ));
    let principal = strict
        .verify(&server.mint(&claims.audience("orders-api"))?)
        .await?;
    assert_eq!(
        principal
            .profile_claim("email_verified")
            .and_then(ProfileClaim::as_bool),
        Some(false)
    );
    assert!(matches!(
        WorkOsVerifier::from_jwks_uri(server.issuer(), "client_01ABC", server.jwks_url())?
            .with_audiences(Vec::<String>::new()),
        Err(baukit_auth::ProviderVerifierError::Configuration(_))
    ));
    Ok(())
}

#[tokio::test]
async fn one_multi_issuer_verifier_routes_oidc_clerk_and_workos_tokens()
-> Result<(), Box<dyn Error>> {
    let oidc_server = MockOidcServer::start().await?;
    let clerk_server = MockOidcServer::start().await?;
    let workos_server = MockOidcServer::start().await?;
    let oidc = OidcVerifier::from_jwks_uri(
        OidcConfig::new(oidc_server.issuer(), "api")?,
        oidc_server.jwks_url(),
    )?;
    let clerk = ClerkVerifier::from_jwks_uri(
        clerk_server.issuer(),
        ["https://app.example.com"],
        clerk_server.jwks_url(),
    )?;
    let workos = WorkOsVerifier::from_jwks_uri(
        workos_server.issuer(),
        "client_01ABC",
        workos_server.jwks_url(),
    )?;
    let verifier = MultiIssuerVerifier::from_verifiers([
        IssuerVerifier::from(oidc.clone()),
        clerk.clone().into(),
        workos.into(),
    ])?;

    let oidc_token = oidc_server.mint(&oidc_server.claims("oidc_user", "api", LIFETIME)?)?;
    let clerk_token = clerk_server.mint(
        &session_claims(&clerk_server, "clerk_user")?.claim("azp", "https://attacker.example"),
    )?;
    let workos_token = workos_server
        .mint(&session_claims(&workos_server, "workos_user")?.claim("client_id", "client_01ABC"))?;

    assert_eq!(verifier.verify(&oidc_token).await?.subject(), "oidc_user");
    assert!(matches!(
        verifier.verify(&clerk_token).await,
        Err(VerificationError::WrongAuthorizedParty)
    ));
    let principal = verifier.verify(&workos_token).await?;
    assert_eq!(principal.issuer(), Some(workos_server.issuer()));
    assert_eq!(principal.client_id(), Some("client_01ABC"));
    assert!(verifier.supports_issuer(clerk.issuer()));

    assert!(matches!(
        MultiIssuerVerifier::from_verifiers([IssuerVerifier::from(oidc.clone()), oidc.into()]),
        Err(MultiIssuerError::DuplicateIssuer(_))
    ));
    assert!(matches!(
        MultiIssuerVerifier::from_verifiers(Vec::<IssuerVerifier>::new()),
        Err(MultiIssuerError::NoIssuers)
    ));
    Ok(())
}
