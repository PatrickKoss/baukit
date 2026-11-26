#![no_main]

//! Structure-aware JWT fuzzing against an independent reference model.
//!
//! Each input picks a verification profile and a claim set, signs it with a
//! fixed Ed25519 key, and checks that the verifier's outcome, including the
//! error kind, equals `expected`. Keys come from an in-process JWKS document
//! and time is fixed, so runs are deterministic.

use std::sync::OnceLock;

use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_auth::{OidcConfig, OidcVerifier, SigningAlgorithm, VerificationError};
use libfuzzer_sys::{
    arbitrary::{Result as ArbitraryResult, Unstructured},
    fuzz_target,
};
use serde_json::{Map, Value, json};
use tokio::runtime::Runtime;

const ISSUER: &str = "https://issuer.example";
const KID: &str = "fuzz-key";
const NOW: u64 = 1_800_000_000;
const SKEW: u64 = 60;
const NONCE: &str = "n1";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Profile {
    Oidc,
    OidcTyped,
    OidcWithClient,
    OidcAllowList,
    Clerk { allow_missing: bool },
    ClientBound,
    ScopeBound,
    IdToken,
}

const PROFILES: [Profile; 9] = [
    Profile::Oidc,
    Profile::OidcTyped,
    Profile::OidcWithClient,
    Profile::OidcAllowList,
    Profile::Clerk {
        allow_missing: false,
    },
    Profile::Clerk {
        allow_missing: true,
    },
    Profile::ClientBound,
    Profile::ScopeBound,
    Profile::IdToken,
];

struct Harness {
    runtime: Runtime,
    key: Ed25519KeyPair,
    verifiers: Vec<OidcVerifier>,
}

fn config(profile: Profile) -> OidcConfig {
    let base = || OidcConfig::new(ISSUER, "api").expect("config");
    let config = match profile {
        Profile::Oidc => base(),
        Profile::OidcTyped => base().requiring_access_token_type(),
        Profile::OidcWithClient | Profile::IdToken => base().with_client_id("web").expect("client"),
        Profile::OidcAllowList => base().with_allowed_clients(["web"]).expect("clients"),
        Profile::Clerk { allow_missing } => {
            OidcConfig::fuzz_clerk(ISSUER, &["https://app"], allow_missing).expect("clerk")
        }
        Profile::ClientBound => OidcConfig::fuzz_client_bound(ISSUER, "cli").expect("client bound"),
        Profile::ScopeBound => {
            OidcConfig::fuzz_scope_bound(ISSUER, &["read"]).expect("scope bound")
        }
    };
    config
        .with_allowed_algorithms([SigningAlgorithm::EdDsa])
        .expect("algorithms")
        .with_clock_skew(std::time::Duration::from_secs(SKEW))
}

fn harness() -> &'static Harness {
    static HARNESS: OnceLock<Harness> = OnceLock::new();
    HARNESS.get_or_init(|| {
        let key = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).expect("key");
        let x = URL_SAFE_NO_PAD.encode(key.public_key().as_ref());
        let jwks =
            json!({"keys": [{"kty": "OKP", "crv": "Ed25519", "kid": KID, "x": x}]}).to_string();
        let verifiers = PROFILES
            .iter()
            .map(|profile| {
                OidcVerifier::from_jwks_document(config(*profile), &jwks).expect("verifier")
            })
            .collect();
        Harness {
            runtime: Runtime::new().expect("runtime"),
            key,
            verifiers,
        }
    })
}

fn pick<T: Clone>(input: &mut Unstructured<'_>, options: &[T]) -> ArbitraryResult<T> {
    Ok(input.choose(options)?.clone())
}

fn time_claim(input: &mut Unstructured<'_>) -> ArbitraryResult<Option<u64>> {
    if input.arbitrary()? {
        return Ok(None);
    }
    let offset = input.int_in_range(-200_i64..=200)?;
    Ok(Some(NOW.saturating_add_signed(offset)))
}

struct Case {
    profile_index: usize,
    typ: Option<&'static str>,
    kid: Option<&'static str>,
    tampered: bool,
    claims: Map<String, Value>,
}

fn case(input: &mut Unstructured<'_>) -> ArbitraryResult<Case> {
    let profile_index = input.choose_index(PROFILES.len())?;
    let typ = pick(
        input,
        &[
            None,
            Some("JWT"),
            Some("at+jwt"),
            Some("application/at+jwt"),
        ],
    )?;
    let kid = pick(input, &[Some(KID), Some(KID), Some("other"), None])?;
    let tampered = input.ratio(1, 16)?;
    let values = [
        Value::Null,
        json!("api"),
        json!("web"),
        json!("cli"),
        json!("https://app"),
        json!("other"),
        json!(""),
        json!(7),
        json!([]),
        json!(["api"]),
        json!(["api", "web"]),
        json!(["web", "other"]),
    ];
    let mut claims = Map::new();
    let mut put = |name: &str, value: Value| {
        if !value.is_null() {
            claims.insert(name.to_owned(), value);
        }
    };
    put(
        "sub",
        pick(
            input,
            &[json!("user"), json!("user"), json!(""), Value::Null],
        )?,
    );
    put(
        "iss",
        pick(
            input,
            &[
                json!(ISSUER),
                json!(ISSUER),
                json!("https://evil"),
                Value::Null,
            ],
        )?,
    );
    put("aud", pick(input, &values)?);
    put("azp", pick(input, &values[..8])?);
    put("client_id", pick(input, &values[..8])?);
    put(
        "scope",
        pick(
            input,
            &[
                Value::Null,
                json!("read"),
                json!("read write"),
                json!("write"),
                json!(["read"]),
                json!(3),
            ],
        )?,
    );
    put(
        "nonce",
        pick(
            input,
            &[Value::Null, Value::Null, json!(NONCE), json!("bad")],
        )?,
    );
    put("exp", time_claim(input)?.map_or(Value::Null, Value::from));
    put("nbf", time_claim(input)?.map_or(Value::Null, Value::from));
    Ok(Case {
        profile_index,
        typ,
        kid,
        tampered,
        claims,
    })
}

fn sign(key: &Ed25519KeyPair, case: &Case) -> String {
    let mut header = json!({"alg": "EdDSA"});
    if let Some(typ) = case.typ {
        header["typ"] = json!(typ);
    }
    if let Some(kid) = case.kid {
        header["kid"] = json!(kid);
    }
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(Value::Object(case.claims.clone()).to_string())
    );
    let mut signature = key.sign(input.as_bytes()).as_ref().to_vec();
    if case.tampered
        && let Some(byte) = signature.first_mut()
    {
        *byte ^= 1;
    }
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature))
}

type Outcome = Result<(), &'static str>;

fn audiences(claims: &Map<String, Value>) -> Vec<&str> {
    match claims.get("aud") {
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn is_string(value: Option<&Value>, expected: &str) -> bool {
    value.and_then(Value::as_str) == Some(expected)
}

fn client_id_rule(value: Option<&Value>, expected: Option<&str>) -> Outcome {
    match (value, expected) {
        (Some(Value::String(id)), Some(expected)) if id == expected => Ok(()),
        (Some(Value::String(id)), None) if !id.is_empty() => Ok(()),
        (Some(Value::String(_)) | None, _) => Err("WrongClientId"),
        (Some(_), _) => Err("InvalidPrincipalContext"),
    }
}

fn scopes(value: Option<&Value>) -> Result<Vec<&str>, &'static str> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::String(scopes)) => Ok(scopes.split_ascii_whitespace().collect()),
        Some(Value::Array(scopes)) => scopes
            .iter()
            .map(|scope| {
                scope
                    .as_str()
                    .filter(|scope| !scope.is_empty())
                    .ok_or("InvalidPrincipalContext")
            })
            .collect(),
        Some(_) => Err("InvalidPrincipalContext"),
    }
}

fn profile_rule(profile: Profile, claims: &Map<String, Value>) -> Outcome {
    let audiences = audiences(claims);
    let several = audiences.len() > 1;
    let azp = claims.get("azp");
    if profile == Profile::IdToken {
        if !audiences.contains(&"web") {
            return Err("WrongAudience");
        }
        return match azp {
            None if !several => Ok(()),
            Some(Value::String(azp)) if azp == "web" => Ok(()),
            _ => Err("WrongAuthorizedParty"),
        };
    }
    if claims.contains_key("nonce") {
        return Err("WrongTokenType");
    }
    match profile {
        Profile::Oidc | Profile::OidcTyped | Profile::OidcWithClient | Profile::OidcAllowList => {
            if !audiences.contains(&"api") {
                return Err("WrongAudience");
            }
            let allowed = match profile {
                Profile::OidcAllowList => is_string(azp, "web"),
                _ => !several || matches!(azp, Some(Value::String(_))),
            };
            if allowed {
                Ok(())
            } else {
                Err("WrongAuthorizedParty")
            }
        }
        Profile::Clerk { allow_missing } => match azp {
            None => {
                if allow_missing {
                    Ok(())
                } else {
                    Err("WrongAuthorizedParty")
                }
            }
            Some(Value::String(azp)) if azp == "https://app" => Ok(()),
            Some(Value::String(_)) => Err("WrongAuthorizedParty"),
            Some(_) => Err("InvalidPrincipalContext"),
        },
        Profile::ClientBound => client_id_rule(claims.get("client_id"), Some("cli")),
        Profile::ScopeBound => {
            client_id_rule(claims.get("client_id"), None)?;
            if scopes(claims.get("scope"))?.contains(&"read") {
                Ok(())
            } else {
                Err("MissingScope")
            }
        }
        Profile::IdToken => Ok(()),
    }
}

fn expected(profile: Profile, case: &Case) -> Outcome {
    let claims = &case.claims;
    let access = profile != Profile::IdToken;
    let aud_shape = match claims.get("aud") {
        None | Some(Value::String(_)) => true,
        Some(Value::Array(many)) => many.iter().all(Value::is_string),
        Some(_) => false,
    };
    if !aud_shape {
        return Err("MalformedToken");
    }
    if access
        && profile == Profile::OidcTyped
        && !matches!(case.typ, Some("at+jwt" | "application/at+jwt"))
    {
        return Err("WrongTokenType");
    }
    match case.kid {
        None => return Err("MissingKeyId"),
        Some(KID) => {}
        Some(_) => return Err("UnknownKeyId"),
    }
    if case.tampered {
        return Err("InvalidSignature");
    }
    if !access && !is_string(claims.get("nonce"), NONCE) {
        return Err("WrongNonce");
    }
    if !claims
        .get("sub")
        .and_then(Value::as_str)
        .is_some_and(|sub| !sub.is_empty())
    {
        return Err("MissingSubject");
    }
    if !is_string(claims.get("iss"), ISSUER) {
        return Err("WrongIssuer");
    }
    profile_rule(profile, claims)?;
    let exp = claims
        .get("exp")
        .and_then(Value::as_u64)
        .ok_or("MissingExpiry")?;
    if NOW > exp + SKEW {
        return Err("Expired");
    }
    if claims
        .get("nbf")
        .and_then(Value::as_u64)
        .is_some_and(|nbf| nbf > NOW + SKEW)
    {
        return Err("NotYetValid");
    }
    if matches!(profile, Profile::ClientBound | Profile::ScopeBound) {
        client_id_rule(claims.get("client_id"), None).map_err(|_| "InvalidPrincipalContext")?;
    }
    scopes(claims.get("scope"))?;
    Ok(())
}

fn kind(result: Result<baukit_auth::Principal, VerificationError>) -> Outcome {
    result.map(|_| ()).map_err(|error| match error {
        VerificationError::MissingKeyId => "MissingKeyId",
        VerificationError::UnknownKeyId => "UnknownKeyId",
        VerificationError::InvalidSignature => "InvalidSignature",
        VerificationError::WrongNonce => "WrongNonce",
        VerificationError::MissingSubject => "MissingSubject",
        VerificationError::WrongIssuer => "WrongIssuer",
        VerificationError::WrongAudience => "WrongAudience",
        VerificationError::WrongAuthorizedParty => "WrongAuthorizedParty",
        VerificationError::WrongTokenType => "WrongTokenType",
        VerificationError::WrongClientId => "WrongClientId",
        VerificationError::MissingScope => "MissingScope",
        VerificationError::MissingExpiry => "MissingExpiry",
        VerificationError::Expired => "Expired",
        VerificationError::NotYetValid => "NotYetValid",
        VerificationError::InvalidPrincipalContext => "InvalidPrincipalContext",
        VerificationError::MalformedToken => "MalformedToken",
        _ => "Other",
    })
}

fuzz_target!(|data: &[u8]| {
    let mut input = Unstructured::new(data);
    let Ok(case) = case(&mut input) else {
        return;
    };
    let harness = harness();
    let (Some(profile), Some(verifier)) = (
        PROFILES.get(case.profile_index),
        harness.verifiers.get(case.profile_index),
    ) else {
        return;
    };
    let token = sign(&harness.key, &case);
    let result = harness.runtime.block_on(async {
        if *profile == Profile::IdToken {
            verifier.verify_id_token_at(&token, NONCE, NOW).await
        } else {
            verifier.verify_at(&token, NOW).await
        }
    });
    assert_eq!(
        kind(result),
        expected(*profile, &case),
        "{:?} {:?}",
        profile,
        case.claims
    );
});
