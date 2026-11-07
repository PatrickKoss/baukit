use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_auth::{
    ClerkMcpVerifier, ClerkVerifier, IdentityVerifier, OidcConfig, OidcVerifier, VerificationError,
    WorkOsVerifier,
};
use baukit_test::{JwtClaims, MockOidcServer, hs256_token, unsigned_token};
use std::{
    error::Error,
    mem::{Discriminant, discriminant},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const LIFETIME: Duration = Duration::from_secs(300);
const FUTURE_OFFSET_SECS: u64 = 3600;

type Attack = (&'static str, String, Discriminant<VerificationError>);

struct Profile {
    name: &'static str,
    verifier: Box<dyn IdentityVerifier>,
    valid: JwtClaims,
    binding_claim: &'static str,
    foreign_binding: &'static str,
    wrong_binding: Discriminant<VerificationError>,
    checks_audience: bool,
}

fn profiles(server: &MockOidcServer) -> Result<Vec<Profile>, Box<dyn Error>> {
    let base = server.claims("user_123", "api", LIFETIME)?;
    Ok(vec![
        Profile {
            name: "oidc allowlist",
            verifier: Box::new(OidcVerifier::from_jwks_uri(
                OidcConfig::new(server.issuer(), "api")?.with_allowed_clients(["web"])?,
                server.jwks_url(),
            )?),
            valid: base.clone().claim("azp", "web"),
            binding_claim: "azp",
            foreign_binding: "foreign_client",
            wrong_binding: discriminant(&VerificationError::WrongAuthorizedParty),
            checks_audience: true,
        },
        Profile {
            name: "clerk session",
            verifier: Box::new(
                ClerkVerifier::from_jwks_uri(
                    server.issuer(),
                    ["https://app.example"],
                    server.jwks_url(),
                )?
                .with_audiences(["api"])?,
            ),
            valid: base.clone().claim("azp", "https://app.example"),
            binding_claim: "azp",
            foreign_binding: "foreign_client",
            wrong_binding: discriminant(&VerificationError::WrongAuthorizedParty),
            checks_audience: true,
        },
        Profile {
            name: "client bound",
            verifier: Box::new(
                WorkOsVerifier::from_jwks_uri(server.issuer(), "client_app", server.jwks_url())?
                    .with_audiences(["api"])?,
            ),
            valid: base.clone().claim("client_id", "client_app"),
            binding_claim: "client_id",
            foreign_binding: "foreign_client",
            wrong_binding: discriminant(&VerificationError::WrongClientId),
            checks_audience: true,
        },
        Profile {
            name: "scope bound",
            verifier: Box::new(ClerkMcpVerifier::from_jwks_uri(
                server.issuer(),
                ["items:read"],
                server.jwks_url(),
            )?),
            valid: base
                .claim("client_id", "client_any")
                .claim("scope", "items:read"),
            binding_claim: "client_id",
            foreign_binding: "",
            wrong_binding: discriminant(&VerificationError::WrongClientId),
            checks_audience: false,
        },
    ])
}

fn without(claims: &JwtClaims, name: &str) -> JwtClaims {
    let mut claims = claims.clone();
    claims.extra.remove(name);
    claims
}

fn tampered(token: &str, claims: &JwtClaims) -> Result<String, Box<dyn Error>> {
    let mut parts = token.split('.');
    let (Some(header), Some(_), Some(signature)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err("minted token is not a three-part JWT".into());
    };
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims.clone().subject("admin"))?);
    Ok(format!("{header}.{payload}.{signature}"))
}

fn attacks(
    server: &MockOidcServer,
    profile: &Profile,
    public_jwks: &[u8],
) -> Result<Vec<Attack>, Box<dyn Error>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let valid = &profile.valid;
    let mut wrong_binding = valid.clone();
    wrong_binding
        .extra
        .insert(profile.binding_claim.into(), profile.foreign_binding.into());
    let mint = |claims: &JwtClaims| server.mint(claims);
    let mut attacks = vec![
        (
            "wrong issuer",
            mint(&valid.clone().issuer("https://other.example"))?,
            discriminant(&VerificationError::WrongIssuer),
        ),
        (
            "expired",
            mint(&valid.clone().expires_at(0))?,
            discriminant(&VerificationError::Expired),
        ),
        (
            "not yet valid",
            mint(&valid.clone().not_before(now + FUTURE_OFFSET_SECS))?,
            discriminant(&VerificationError::NotYetValid),
        ),
        (
            "foreign binding",
            mint(&wrong_binding)?,
            profile.wrong_binding,
        ),
        (
            "missing binding",
            mint(&without(valid, profile.binding_claim))?,
            profile.wrong_binding,
        ),
        (
            "alg none",
            unsigned_token(valid)?,
            discriminant(&VerificationError::InvalidSignature),
        ),
        (
            "hs256 with public key",
            hs256_token(public_jwks, valid)?,
            discriminant(&VerificationError::DisallowedAlgorithm),
        ),
        (
            "unknown kid",
            server.mint_with_key_id(valid, "unpublished")?,
            discriminant(&VerificationError::UnknownKeyId),
        ),
        (
            "tampered payload",
            tampered(&mint(valid)?, valid)?,
            discriminant(&VerificationError::InvalidSignature),
        ),
    ];
    if profile.checks_audience {
        attacks.push((
            "wrong audience",
            mint(&valid.clone().audience("other"))?,
            discriminant(&VerificationError::WrongAudience),
        ));
    }
    Ok(attacks)
}

#[tokio::test]
async fn every_token_profile_rejects_forged_foreign_and_stale_tokens() -> Result<(), Box<dyn Error>>
{
    let server = MockOidcServer::start().await?;
    let public_jwks = reqwest::get(server.jwks_url()).await?.bytes().await?;
    for profile in profiles(&server)? {
        profile
            .verifier
            .verify(&server.mint(&profile.valid)?)
            .await?;
        for (attack, token, expected) in attacks(&server, &profile, &public_jwks)? {
            let error = profile
                .verifier
                .verify(&token)
                .await
                .err()
                .ok_or_else(|| format!("{} / {attack}: token accepted", profile.name))?;
            assert_eq!(
                discriminant(&error),
                expected,
                "{} / {attack}: {error:?}",
                profile.name
            );
        }
    }
    Ok(())
}
