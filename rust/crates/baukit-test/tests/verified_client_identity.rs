use std::{error::Error, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_auth::{OidcConfig, OidcVerifier, PrincipalClaimMapping, VerificationError};
use baukit_test::{MockOidcServer, hs256_token, unsigned_token};
use serde_json::{Value, json};

async fn verifier(server: &MockOidcServer) -> Result<OidcVerifier, Box<dyn Error>> {
    Ok(OidcVerifier::discover(
        OidcConfig::new(server.issuer(), "api")?
            .with_clock_skew(Duration::ZERO)
            .with_principal_claims(PrincipalClaimMapping::new().client_id_claim("azp")),
    )
    .await?)
}

#[tokio::test]
async fn maps_client_and_tenancy_fields_independently() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = OidcVerifier::discover(
        OidcConfig::new(server.issuer(), "api")?.with_principal_claims(
            PrincipalClaimMapping::new()
                .organization_claim("org_id")
                .tenant_claim("tenant_id")
                .client_id_claim("authorized_client"),
        ),
    )
    .await?;
    let claims = server
        .claims("subject", "api", Duration::from_secs(300))?
        .claim("authorized_client", " Mobile-Ä ")
        .claim("azp", "unconfigured-client")
        .claim("org_id", "org")
        .claim("tenant_id", "tenant");
    let principal = verifier.verify(&server.mint(&claims)?).await?;
    assert_eq!(principal.client_id(), Some(" Mobile-Ä "));
    assert_eq!(principal.organization(), Some("org"));
    assert_eq!(principal.tenant(), Some("tenant"));
    assert_eq!(principal.subject(), "subject");
    assert_eq!(principal.issuer(), Some(server.issuer()));
    assert!(principal.api_token().is_none());
    Ok(())
}

#[tokio::test]
async fn missing_null_and_unconfigured_claims_have_no_client_identity() -> Result<(), Box<dyn Error>>
{
    let server = MockOidcServer::start().await?;
    let verifier = verifier(&server).await?;
    let claims = server.claims("subject", "api", Duration::from_secs(300))?;
    for claims in [claims.clone(), claims.clone().claim("azp", Value::Null)] {
        assert!(
            verifier
                .verify(&server.mint(&claims)?)
                .await?
                .client_id()
                .is_none()
        );
    }
    let unmapped = OidcVerifier::discover(OidcConfig::new(server.issuer(), "api")?).await?;
    for value in [json!("mobile"), json!(["mobile"])] {
        let token = server.mint(&claims.clone().claim("azp", value))?;
        assert!(unmapped.verify(&token).await?.client_id().is_none());
    }
    Ok(())
}

#[tokio::test]
async fn malformed_mapped_client_claims_fail_verification() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = verifier(&server).await?;
    for value in [
        json!(""),
        json!([]),
        json!({}),
        json!(true),
        json!(false),
        json!(42),
        json!(1.5),
    ] {
        let claims = server
            .claims("subject", "api", Duration::from_secs(300))?
            .claim("azp", value);
        assert!(matches!(
            verifier.verify(&server.mint(&claims)?).await,
            Err(VerificationError::InvalidPrincipalContext)
        ));
    }
    Ok(())
}

#[tokio::test]
async fn changed_client_claim_without_new_signature_is_rejected() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = verifier(&server).await?;
    let claims = server
        .claims("subject", "api", Duration::from_secs(300))?
        .claim("azp", "cli");
    let token = server.mint(&claims)?;
    let mut parts = token.split('.');
    let header = parts.next().ok_or("missing header")?;
    let payload = parts.next().ok_or("missing payload")?;
    let signature = parts.next().ok_or("missing signature")?;
    let mut payload: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload)?)?;
    payload["azp"] = json!("mobile");
    let changed = format!(
        "{header}.{}.{signature}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload)?)
    );
    assert!(matches!(
        verifier.verify(&changed).await,
        Err(VerificationError::InvalidSignature)
    ));
    assert_eq!(verifier.verify(&token).await?.client_id(), Some("cli"));
    Ok(())
}

#[tokio::test]
async fn client_mapping_does_not_bypass_registered_claim_algorithm_or_key_checks()
-> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = verifier(&server).await?;
    let claims = server
        .claims("subject", "api", Duration::from_secs(300))?
        .claim("azp", "mobile");
    assert!(matches!(
        verifier
            .verify(&server.mint(&claims.clone().issuer("https://wrong.invalid"))?)
            .await,
        Err(VerificationError::WrongIssuer)
    ));
    assert!(matches!(
        verifier
            .verify(&server.mint(&claims.clone().audience("wrong"))?)
            .await,
        Err(VerificationError::WrongAudience)
    ));
    assert!(matches!(
        verifier
            .verify(&server.mint(&claims.clone().expires_at(1))?)
            .await,
        Err(VerificationError::Expired)
    ));
    assert!(matches!(
        verifier
            .verify(&server.mint(&claims.clone().not_before(u64::MAX))?)
            .await,
        Err(VerificationError::NotYetValid)
    ));
    assert!(verifier.verify(&unsigned_token(&claims)?).await.is_err());
    assert!(matches!(
        verifier
            .verify(&hs256_token(b"fixture-only-secret", &claims)?)
            .await,
        Err(VerificationError::DisallowedAlgorithm)
    ));
    let signed = server.mint(&claims)?;
    let mut parts = signed.split('.');
    let header = parts.next().ok_or("missing header")?;
    let payload = parts.next().ok_or("missing payload")?;
    let signature = parts.next().ok_or("missing signature")?;
    let mut header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header)?)?;
    header["kid"] = json!("unknown-client-mapping-key");
    let unknown_key = format!(
        "{}.{payload}.{signature}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header)?)
    );
    assert!(matches!(
        verifier.verify(&unknown_key).await,
        Err(VerificationError::UnknownKeyId)
    ));
    Ok(())
}
