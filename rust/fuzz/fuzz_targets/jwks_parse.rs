#![no_main]

//! Z-02: arbitrary JWKS documents.
//!
//! Raw bytes must never panic the parser. For structured documents, a token
//! signed by the fixed Ed25519 key verifies iff the first parseable key with
//! its `kid` is that key with a matching `kty`, `crv` and `alg`.

use std::sync::OnceLock;

use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_auth::{OidcConfig, OidcVerifier, SigningAlgorithm};
use libfuzzer_sys::{
    arbitrary::{Result as ArbitraryResult, Unstructured},
    fuzz_target,
};
use serde_json::{Map, Value, json};
use tokio::runtime::Runtime;

const ISSUER: &str = "https://issuer.example";
const KID: &str = "signer";
const NOW: u64 = 1_800_000_000;

struct Harness {
    runtime: Runtime,
    signer: Ed25519KeyPair,
    other: Ed25519KeyPair,
}

fn harness() -> &'static Harness {
    static HARNESS: OnceLock<Harness> = OnceLock::new();
    HARNESS.get_or_init(|| Harness {
        runtime: Runtime::new().expect("runtime"),
        signer: Ed25519KeyPair::from_seed_unchecked(&[1; 32]).expect("signer"),
        other: Ed25519KeyPair::from_seed_unchecked(&[2; 32]).expect("other"),
    })
}

fn config() -> OidcConfig {
    OidcConfig::new(ISSUER, "api")
        .expect("config")
        .with_allowed_algorithms([SigningAlgorithm::EdDsa])
        .expect("algorithms")
}

fn token(signer: &Ed25519KeyPair) -> String {
    let header = json!({"alg": "EdDSA", "kid": KID});
    let claims = json!({"sub": "u", "iss": ISSUER, "aud": "api", "exp": NOW + 600});
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let signature = URL_SAFE_NO_PAD.encode(signer.sign(input.as_bytes()));
    format!("{input}.{signature}")
}

struct Key {
    value: Map<String, Value>,
    parseable: bool,
    usable: bool,
}

fn optional(
    input: &mut Unstructured<'_>,
    key: &mut Map<String, Value>,
    name: &str,
    options: &[&str],
) -> ArbitraryResult<Option<String>> {
    let index = input.choose_index(options.len() + 1)?;
    let value = options.get(index).map(|value| (*value).to_owned());
    if let Some(value) = &value {
        key.insert(name.to_owned(), json!(value));
    }
    Ok(value)
}

fn key(input: &mut Unstructured<'_>, harness: &Harness) -> ArbitraryResult<Key> {
    let signer_x = URL_SAFE_NO_PAD.encode(harness.signer.public_key().as_ref());
    let other_x = URL_SAFE_NO_PAD.encode(harness.other.public_key().as_ref());
    let mut value = Map::new();
    let kid = optional(input, &mut value, "kid", &[KID, "other"])?;
    let kty = optional(input, &mut value, "kty", &["OKP", "RSA", "EC"])?;
    let crv = optional(input, &mut value, "crv", &["Ed25519", "P-256"])?;
    let x = optional(input, &mut value, "x", &[&signer_x, &other_x, "!!", ""])?;
    let alg = optional(input, &mut value, "alg", &["EdDSA", "RS256"])?;
    let usage = optional(input, &mut value, "use", &["sig", "enc"])?;
    let parseable = kty.is_some();
    let usable = kid.as_deref() == Some(KID)
        && kty.as_deref() == Some("OKP")
        && crv.as_deref() == Some("Ed25519")
        && x.as_deref() == Some(signer_x.as_str())
        && alg.as_deref().is_none_or(|alg| alg == "EdDSA")
        && usage.as_deref().is_none_or(|usage| usage == "sig");
    Ok(Key {
        value,
        parseable,
        usable,
    })
}

fn structured(input: &mut Unstructured<'_>, harness: &Harness) -> ArbitraryResult<()> {
    let count = input.int_in_range(0..=4)?;
    let mut keys = Vec::new();
    for _ in 0..count {
        keys.push(key(input, harness)?);
    }
    let selected = keys
        .iter()
        .find(|key| key.parseable && key.value.get("kid").and_then(Value::as_str) == Some(KID));
    let expected = selected.is_some_and(|key| key.usable);
    let document = json!({"keys": keys.iter().map(|key| Value::Object(key.value.clone())).collect::<Vec<_>>()});
    let verifier =
        OidcVerifier::from_jwks_document(config(), &document.to_string()).expect("structured JWKS");
    let verified = harness
        .runtime
        .block_on(verifier.verify_at(&token(&harness.signer), NOW));
    assert_eq!(verified.is_ok(), expected, "{document}: {verified:?}");
    Ok(())
}

fuzz_target!(|data: &[u8]| {
    let harness = harness();
    let Some((mode, rest)) = data.split_first() else {
        return;
    };
    if mode % 2 == 0 {
        let Ok(text) = std::str::from_utf8(rest) else {
            return;
        };
        if let Ok(verifier) = OidcVerifier::from_jwks_document(config(), text) {
            let _ = harness
                .runtime
                .block_on(verifier.verify_at(&token(&harness.signer), NOW));
        }
        return;
    }
    let _ = structured(&mut Unstructured::new(rest), harness);
});
