use std::{error::Error, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_auth::{OidcConfig, OidcConfigError, OidcVerifier, VerificationError};
use baukit_test::MockOidcServer;
use serde_json::json;

#[tokio::test]
async fn id_token_nonce_is_required_and_matches_exactly() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = OidcVerifier::discover(
        OidcConfig::new(server.issuer(), "client")?.with_client_id("client")?,
    )
    .await?;
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
async fn id_tokens_require_an_explicit_client_and_its_audience() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let claims = server
        .claims("user", "api", Duration::from_secs(300))?
        .claim("nonce", "login-nonce");
    let token = server.mint(&claims)?;
    for config in [
        OidcConfig::new(server.issuer(), "api")?,
        OidcConfig::new(server.issuer(), "api")?.with_allowed_clients(["api"])?,
    ] {
        let verifier = OidcVerifier::discover(config).await?;
        assert!(matches!(
            verifier.verify_id_token(&token, "login-nonce").await,
            Err(VerificationError::MissingClientId)
        ));
    }
    let verifier =
        OidcVerifier::discover(OidcConfig::new(server.issuer(), "api")?.with_client_id("web")?)
            .await?;
    assert!(matches!(
        verifier.verify_id_token(&token, "login-nonce").await,
        Err(VerificationError::WrongAudience)
    ));
    let token = server.mint(&claims.audience("web"))?;
    assert_eq!(
        verifier
            .verify_id_token(&token, "login-nonce")
            .await?
            .subject(),
        "user"
    );
    Ok(())
}

#[tokio::test]
async fn id_token_authorized_party_must_match_its_client() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = OidcVerifier::discover(
        OidcConfig::new(server.issuer(), "api")?
            .with_client_id("web")?
            .with_allowed_clients(["mobile"])?,
    )
    .await?;
    let mut claims = server
        .claims("user", "web", Duration::from_secs(300))?
        .claim("nonce", "login-nonce");
    claims.aud = None;
    for audience in [json!("web"), json!(["web"]), json!(["web", "api"])] {
        let claims = claims.clone().claim("aud", audience.clone());
        let token = server.mint(&claims.clone().claim("azp", "web"))?;
        assert_eq!(
            verifier
                .verify_id_token(&token, "login-nonce")
                .await?
                .subject(),
            "user"
        );
        for azp in [
            json!("mobile"),
            json!("unknown"),
            json!(""),
            json!(null),
            json!(42),
            json!(["web"]),
        ] {
            let token = server.mint(&claims.clone().claim("azp", azp))?;
            assert!(matches!(
                verifier.verify_id_token(&token, "login-nonce").await,
                Err(VerificationError::WrongAuthorizedParty)
            ));
        }
        let result = verifier
            .verify_id_token(&server.mint(&claims)?, "login-nonce")
            .await;
        if audience.as_array().is_some_and(|values| values.len() > 1) {
            assert!(matches!(
                result,
                Err(VerificationError::WrongAuthorizedParty)
            ));
        } else {
            assert_eq!(result?.subject(), "user");
        }
    }
    Ok(())
}

#[tokio::test]
async fn keycloak_access_tokens_accept_public_clients_by_default() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let base_url = server
        .issuer()
        .strip_suffix("/realms/baukit-test")
        .ok_or("unexpected mock issuer")?;
    for config in [
        OidcConfig::new(server.issuer(), "app-backend")?,
        OidcConfig::keycloak(base_url, "baukit-test", "app-backend")?,
    ] {
        let default = OidcVerifier::discover(config.clone()).await?;
        let allowed = OidcVerifier::discover(
            config
                .clone()
                .with_allowed_clients(["app-web", "app-mobile"])?,
        )
        .await?;
        let excluded = OidcVerifier::discover(config.with_allowed_clients(["app-worker"])?).await?;
        for audience in [
            json!("app-backend"),
            json!(["app-backend"]),
            json!(["app-backend", "account"]),
        ] {
            for client in ["app-web", "app-mobile"] {
                let mut claims = server.claims("user", "app-backend", Duration::from_secs(300))?;
                claims.aud = None;
                let token =
                    server.mint(&claims.claim("aud", audience.clone()).claim("azp", client))?;
                assert_eq!(default.verify(&token).await?.subject(), "user");
                assert_eq!(allowed.verify(&token).await?.subject(), "user");
                assert!(matches!(
                    excluded.verify(&token).await,
                    Err(VerificationError::WrongAuthorizedParty)
                ));
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn access_tokens_require_a_string_authorized_party_for_several_audiences()
-> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = OidcVerifier::discover(OidcConfig::new(server.issuer(), "api")?).await?;
    let mut claims = server.claims("user", "api", Duration::from_secs(300))?;
    claims.aud = None;
    let claims = claims.claim("aud", json!(["api", "account"]));
    for invalid in [
        claims.clone(),
        claims.clone().claim("azp", json!(null)),
        claims.clone().claim("azp", json!(42)),
        claims.clone().claim("azp", json!(["web"])),
    ] {
        assert!(matches!(
            verifier.verify(&server.mint(&invalid)?).await,
            Err(VerificationError::WrongAuthorizedParty)
        ));
    }
    let wrong_audience = claims
        .claim("aud", json!(["web", "account"]))
        .claim("azp", "web");
    assert!(matches!(
        verifier.verify(&server.mint(&wrong_audience)?).await,
        Err(VerificationError::WrongAudience)
    ));
    Ok(())
}

#[tokio::test]
async fn access_token_allowlist_requires_a_client_and_is_independent_of_id_token_client()
-> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let config = OidcConfig::new(server.issuer(), "api")?;
    for config in [
        config
            .clone()
            .with_client_id("web")?
            .with_allowed_clients(["mobile"])?,
        config
            .with_allowed_clients(["mobile"])?
            .with_client_id("web")?,
    ] {
        let verifier = OidcVerifier::discover(config).await?;
        for audience in [json!("api"), json!(["api", "account"])] {
            let mut claims = server.claims("user", "api", Duration::from_secs(300))?;
            claims.aud = None;
            let claims = claims.claim("aud", audience);
            assert_eq!(
                verifier
                    .verify(&server.mint(&claims.clone().claim("azp", "mobile"))?)
                    .await?
                    .subject(),
                "user"
            );
            for invalid in [
                claims.clone(),
                claims.clone().claim("azp", "web"),
                claims.clone().claim("azp", json!(null)),
                claims.clone().claim("azp", json!(42)),
            ] {
                assert!(matches!(
                    verifier.verify(&server.mint(&invalid)?).await,
                    Err(VerificationError::WrongAuthorizedParty)
                ));
            }
        }
    }
    let verifier =
        OidcVerifier::discover(OidcConfig::new(server.issuer(), "api")?.with_client_id("web")?)
            .await?;
    let claims = server.claims("user", "api", Duration::from_secs(300))?;
    for valid in [
        claims.clone(),
        claims.clone().claim("azp", json!(null)),
        claims.claim("azp", "mobile"),
    ] {
        assert_eq!(
            verifier.verify(&server.mint(&valid)?).await?.subject(),
            "user"
        );
    }
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
