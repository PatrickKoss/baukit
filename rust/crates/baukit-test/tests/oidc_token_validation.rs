use std::{error::Error, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_auth::{OidcConfig, OidcConfigError, OidcVerifier, VerificationError};
use baukit_test::MockOidcServer;
use serde_json::json;

#[tokio::test]
async fn id_token_nonce_is_required_and_matches_exactly() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = OidcVerifier::discover(OidcConfig::new(server.issuer(), "client")?).await?;
    let claims = server.claims("user", "client", Duration::from_secs(300))?;
    let valid = server.mint(&claims.clone().claim("nonce", "login-nonce"))?;
    assert_eq!(
        verifier
            .verify_id_token(&valid, "login-nonce")
            .await?
            .subject(),
        "user"
    );
    for nonce in ["wrong-nonce", "login-noncE", "", "login-nonce-longer"] {
        assert!(matches!(
            verifier.verify_id_token(&valid, nonce).await,
            Err(VerificationError::WrongNonce)
        ));
    }
    for invalid in [
        claims.clone(),
        claims.clone().claim("nonce", json!(null)),
        claims.clone().claim("nonce", json!(123)),
    ] {
        assert!(matches!(
            verifier
                .verify_id_token(&server.mint(&invalid)?, "login-nonce")
                .await,
            Err(VerificationError::WrongNonce)
        ));
    }
    assert_eq!(
        verifier.verify(&server.mint(&claims)?).await?.subject(),
        "user"
    );
    let expired = server.mint(&claims.claim("nonce", "login-nonce").expires_at(1))?;
    assert!(matches!(
        verifier.verify_id_token(&expired, "login-nonce").await,
        Err(VerificationError::Expired)
    ));
    let (signing_input, signature) = valid.rsplit_once('.').ok_or("missing signature")?;
    let mut signature = URL_SAFE_NO_PAD.decode(signature)?;
    signature[0] ^= 1;
    let tampered = format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(signature));
    assert!(matches!(
        verifier.verify_id_token(&tampered, "login-nonce").await,
        Err(VerificationError::InvalidSignature)
    ));
    Ok(())
}

#[tokio::test]
async fn several_audiences_require_the_configured_client_as_authorized_party()
-> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let config = OidcConfig::new(server.issuer(), "api")?
        .with_client_id("web")?
        .with_allowed_clients(["mobile"])?;
    let verifier = OidcVerifier::discover(config).await?;
    let mut claims = server.claims("user", "api", Duration::from_secs(300))?;
    claims.aud = None;
    let claims = claims.claim("aud", json!(["api", "other-api"]));
    let valid = server.mint(&claims.clone().claim("azp", "web"))?;
    assert_eq!(verifier.verify(&valid).await?.subject(), "user");
    for invalid in [
        claims.clone(),
        claims.clone().claim("azp", "mobile"),
        claims.clone().claim("azp", "unknown"),
        claims.clone().claim("azp", json!(null)),
    ] {
        assert!(matches!(
            verifier.verify(&server.mint(&invalid)?).await,
            Err(VerificationError::WrongAuthorizedParty)
        ));
    }
    let wrong_audience = claims
        .claim("aud", json!(["web", "other-api"]))
        .claim("azp", "web");
    assert!(matches!(
        verifier.verify(&server.mint(&wrong_audience)?).await,
        Err(VerificationError::WrongAudience)
    ));
    Ok(())
}

#[tokio::test]
async fn single_audience_accepts_only_allowed_authorized_parties() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = OidcVerifier::discover(
        OidcConfig::new(server.issuer(), "api")?
            .with_client_id("web")?
            .with_allowed_clients(["mobile"])?,
    )
    .await?;
    let claims = server.claims("user", "api", Duration::from_secs(300))?;
    for valid in [
        claims.clone(),
        claims.clone().claim("azp", "web"),
        claims.clone().claim("azp", "mobile"),
    ] {
        assert_eq!(
            verifier.verify(&server.mint(&valid)?).await?.subject(),
            "user"
        );
    }
    for azp in [
        json!("unknown"),
        json!(""),
        json!(42),
        json!(["web"]),
        json!(null),
    ] {
        assert!(matches!(
            verifier
                .verify(&server.mint(&claims.clone().claim("azp", azp))?)
                .await,
            Err(VerificationError::WrongAuthorizedParty)
        ));
    }
    let default = OidcVerifier::discover(OidcConfig::new(server.issuer(), "api")?).await?;
    assert_eq!(
        default
            .verify(&server.mint(&claims.clone().claim("azp", "api"))?)
            .await?
            .subject(),
        "user"
    );
    assert!(matches!(
        default
            .verify(&server.mint(&claims.claim("azp", "web"))?)
            .await,
        Err(VerificationError::WrongAuthorizedParty)
    ));
    Ok(())
}

#[test]
fn client_configuration_rejects_empty_values() -> Result<(), Box<dyn Error>> {
    let config = OidcConfig::new("https://issuer.example", "api")?;
    assert!(matches!(
        config.clone().with_client_id(""),
        Err(OidcConfigError::EmptyValue("client ID"))
    ));
    assert!(config.clone().with_allowed_clients([""]).is_err());
    assert!(config.with_allowed_clients(Vec::<String>::new()).is_err());
    Ok(())
}
