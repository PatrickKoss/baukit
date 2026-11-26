use proptest::prelude::*;
use serde_json::json;

use super::*;

const ISSUER: &str = "https://issuer.example";

#[derive(Clone, Copy, Debug)]
enum Profile {
    Oidc,
    OidcWithClient,
    OidcAllowList,
    Clerk { allow_missing: bool },
    ClientBound,
    ScopeBound,
}

fn config(profile: Profile) -> OidcConfig {
    let base = || OidcConfig::new(ISSUER, "api").expect("config");
    match profile {
        Profile::Oidc => base(),
        Profile::OidcWithClient => base().with_client_id("web").expect("client"),
        Profile::OidcAllowList => base().with_allowed_clients(["web"]).expect("clients"),
        Profile::Clerk { allow_missing } => {
            let config = OidcConfig::clerk(ISSUER, ["https://app"]).expect("clerk");
            if allow_missing {
                config.allowing_missing_authorized_party()
            } else {
                config
            }
        }
        Profile::ClientBound => OidcConfig::client_bound(ISSUER, "cli").expect("client bound"),
        Profile::ScopeBound => OidcConfig::scope_bound(ISSUER, ["read"]).expect("scope bound"),
    }
}

fn audience_values(aud: &Value) -> Vec<&str> {
    match aud {
        Value::String(one) => vec![one.as_str()],
        Value::Array(many) => many.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

/// Independent statement of the access-token rules, kept as a flat table.
fn expected(
    profile: Profile,
    aud: &Value,
    azp: &Value,
    client_id: &Value,
    nonce: bool,
) -> &'static str {
    if nonce {
        return "WrongTokenType";
    }
    let audiences = audience_values(aud);
    let has_audience_rule = !matches!(
        profile,
        Profile::Clerk { .. } | Profile::ClientBound | Profile::ScopeBound
    );
    if has_audience_rule && !audiences.contains(&"api") {
        return "WrongAudience";
    }
    let several = audiences.len() > 1;
    match (profile, azp) {
        (Profile::OidcAllowList, Value::String(azp)) if azp == "web" => "Ok",
        (Profile::OidcAllowList, _) => "WrongAuthorizedParty",
        (Profile::Oidc | Profile::OidcWithClient, Value::String(_)) => "Ok",
        (Profile::Oidc | Profile::OidcWithClient, _) if !several => "Ok",
        (Profile::Oidc | Profile::OidcWithClient, _) => "WrongAuthorizedParty",
        (
            Profile::Clerk {
                allow_missing: true,
            },
            Value::Null,
        ) => "Ok",
        (Profile::Clerk { .. }, Value::Null) => "WrongAuthorizedParty",
        (Profile::Clerk { .. }, Value::String(azp)) if azp == "https://app" => "Ok",
        (Profile::Clerk { .. }, Value::String(_)) => "WrongAuthorizedParty",
        (Profile::Clerk { .. }, _) => "InvalidPrincipalContext",
        (Profile::ClientBound, _) => match client_id {
            Value::String(id) if id == "cli" => "Ok",
            Value::String(_) | Value::Null => "WrongClientId",
            _ => "InvalidPrincipalContext",
        },
        (Profile::ScopeBound, _) => match client_id {
            Value::String(id) if !id.is_empty() => "Ok",
            Value::String(_) | Value::Null => "WrongClientId",
            _ => "InvalidPrincipalContext",
        },
    }
}

fn profile() -> impl Strategy<Value = Profile> {
    prop_oneof![
        Just(Profile::Oidc),
        Just(Profile::OidcWithClient),
        Just(Profile::OidcAllowList),
        any::<bool>().prop_map(|allow_missing| Profile::Clerk { allow_missing }),
        Just(Profile::ClientBound),
        Just(Profile::ScopeBound),
    ]
}

fn audience() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        Just(json!("api")),
        Just(json!("other")),
        Just(json!([])),
        Just(json!(["api"])),
        Just(json!(["api", "other"])),
        Just(json!(["other", "more"])),
    ]
}

fn party() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        Just(json!("web")),
        Just(json!("cli")),
        Just(json!("https://app")),
        Just(json!("")),
        Just(json!(7)),
    ]
}

proptest! {
    /// P-05: `validate_token_profile` equals the reference table for every
    /// combination of audience shape, `azp`, `client_id` and profile.
    #[test]
    fn profile_validation_matches_reference_table(
        profile in profile(),
        aud in audience(),
        azp in party(),
        client_id in party(),
        nonce in any::<bool>(),
    ) {
        let mut claims = json!({"sub": "user", "iss": ISSUER, "exp": 1, "scope": "read"});
        for (name, value) in [("aud", &aud), ("azp", &azp), ("client_id", &client_id)] {
            if !value.is_null() {
                claims[name] = value.clone();
            }
        }
        if nonce {
            claims["nonce"] = json!("n");
        }
        let claims: JwtClaims = serde_json::from_value(claims).expect("claims");
        let actual = match validate_token_profile(&claims, &config(profile), TokenVerification::Access) {
            Ok(()) => "Ok".to_owned(),
            Err(error) => format!("{error:?}"),
        };
        prop_assert_eq!(actual, expected(profile, &aud, &azp, &client_id, nonce));
    }
}

#[test]
fn access_tokens_with_several_audiences_check_azp_only_with_an_allow_list() {
    let claims: JwtClaims = serde_json::from_value(
        json!({"sub": "u", "iss": ISSUER, "exp": 1, "aud": ["api", "account"], "azp": "app-web"}),
    )
    .expect("claims");
    let verdict =
        |profile| validate_token_profile(&claims, &config(profile), TokenVerification::Access);
    assert!(verdict(Profile::Oidc).is_ok());
    assert!(verdict(Profile::OidcWithClient).is_ok());
    assert!(matches!(
        verdict(Profile::OidcAllowList),
        Err(VerificationError::WrongAuthorizedParty)
    ));
}
