//! Writes `fixtures/suite/generated-v1.json`: ticket texts (D-03) and return URLs
//! (D-05) with the Rust verdict, for `@baukit/suite-client` to match.

use aws_lc_rs::signature::Ed25519KeyPair;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_suite::{
    domain::validate_return_url,
    platform::{TicketJwks, verify_ticket},
};
use serde_json::{Value, json};
use std::{error::Error, fs, path::Path};

const TICKETS: &str = include_str!("../../../../fixtures/entitlement-tickets/v1/vectors.json");
const BILLING: &str =
    include_str!("../../../../fixtures/entitlement-tickets/v1/billing-vectors.json");
const OUTPUT: &str = "../../../fixtures/suite/generated-v1.json";
const SEED: u64 = 0x5eed_0003;
const RANDOM_CASES: usize = 64;
const ORIGIN: &str = "https://alpha.example";
const SCHEME: &str = "alpha";

/// A fixed-seed xorshift, so the file is identical on every run.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        let len = u64::try_from(items.len()).unwrap_or(1);
        let index = usize::try_from(self.next() % len).unwrap_or_default();
        items.get(index).copied().unwrap_or_default()
    }
}

fn signed(
    seed: &[u8],
    kid: &str,
    header: Option<&str>,
    body: &str,
) -> Result<String, Box<dyn Error>> {
    let pair = Ed25519KeyPair::from_seed_unchecked(seed).map_err(|e| e.to_string())?;
    let header = header.map_or_else(
        || json!({"alg": "EdDSA", "kid": kid}).to_string(),
        str::to_owned,
    );
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(body)
    );
    let signature = pair.sign(input.as_bytes());
    Ok(format!(
        "{input}.{}",
        URL_SAFE_NO_PAD.encode(signature.as_ref())
    ))
}

fn ticket_bodies(claims: &Value) -> Vec<(String, Option<String>, String)> {
    let body = claims.to_string();
    let exp = claims["exp"].as_i64().unwrap_or_default();
    let mut cases = vec![("canonical".to_owned(), None, body.clone())];
    if let Some(fields) = claims.as_object() {
        for (key, value) in fields {
            let duplicated = format!("{},{}:{}}}", &body[..body.len() - 1], json!(key), value);
            cases.push((format!("duplicate {key}"), None, duplicated));
        }
    }
    for suffix in [".0", "e0", "E0", ".5"] {
        cases.push((
            format!("exp with {suffix}"),
            None,
            body.replace(&format!("\"exp\":{exp}"), &format!("\"exp\":{exp}{suffix}")),
        ));
    }
    for manage in [
        "https://accounts.example.test/plans",
        "HTTPS://accounts.example.test/plans",
        "https://accounts.example.test:65535/plans",
        "https://accounts.example.test:65536/plans",
        "https://accounts.example.test\\@evil.example/plans",
        "https://[::1]/plans",
        "https://user@accounts.example.test/plans",
        "https://accounts.example.test/pl ans",
        "javascript://accounts.example.test/%0aalert(1)",
        "https://accounts.example.test?x=1",
    ] {
        let mut changed = claims.clone();
        if let Some(object) = changed.as_object_mut() {
            object.insert(
                "billing".into(),
                json!({"channel": "web", "manageUrl": manage}),
            );
        }
        cases.push((format!("manageUrl {manage}"), None, changed.to_string()));
    }
    cases.push(("duplicate header alg".into(), Some(String::new()), body));
    cases
}

fn return_urls() -> Vec<String> {
    let fixed = [
        "https://alpha.example/suite/linked",
        "HTTPS://alpha.example/suite/linked",
        "https://alpha.example/suite/linked#x",
        "https://alpha.example/suite/linked?x=1",
        "https://alpha.example/suite/../suite/linked",
        "https://alpha.example\\suite/linked",
        "https://alpha.example./suite/linked",
        "https://alpha.example:443/suite/linked",
        " https://alpha.example/suite/linked",
        "http://alpha.example/suite/linked",
        "alpha://suite/linked",
        "ALPHA://suite/linked",
        "alpha-dev://suite/linked",
        "alphadev://suite/linked",
        "alpha://suite/linked#x",
        "alpha://suite:1/linked",
        "alpha://SUITE/linked",
        "alpha://suite/./linked",
        "https://suite/linked",
    ];
    let parts = [
        "https://",
        "http://",
        "alpha://",
        "alpha-dev://",
        "alpha.example",
        "suite",
        "/suite",
        "/linked",
        "#",
        "?a=1",
        ":443",
        ".",
        "\\",
        "%2F",
        "@x",
    ];
    let mut rng = Rng(SEED);
    let mut urls: Vec<String> = fixed.iter().map(|s| (*s).to_owned()).collect();
    for _ in 0..RANDOM_CASES {
        let count = 2 + usize::try_from(rng.next() % 4).unwrap_or_default();
        urls.push((0..count).map(|_| rng.pick(&parts)).collect());
    }
    urls
}

fn main() -> Result<(), Box<dyn Error>> {
    let tickets: Value = serde_json::from_str(TICKETS)?;
    let billing: Value = serde_json::from_str(BILLING)?;
    let seed = hex(tickets
        .get("testSeedHex")
        .and_then(Value::as_str)
        .ok_or("seed")?)?;
    let jwks_value = billing.get("jwks").ok_or("jwks")?;
    let jwks: TicketJwks = serde_json::from_value(jwks_value.clone())?;
    let kid = jwks.keys.first().ok_or("kid")?.kid.clone();
    let issuer = billing
        .get("issuer")
        .and_then(Value::as_str)
        .ok_or("issuer")?;
    let vector = billing
        .get("vectors")
        .and_then(Value::as_array)
        .and_then(|v| v.iter().find(|v| v["name"] == "billing-web"))
        .ok_or("billing-web vector")?;
    let audience = vector["audience"].as_str().ok_or("audience")?;
    let now = vector["now"].as_i64().ok_or("now")?;
    let duplicate_header = format!(r#"{{"alg":"EdDSA","kid":"{kid}","alg":"EdDSA"}}"#);
    let mut ticket_cases = Vec::new();
    for (name, header, body) in ticket_bodies(&vector["claims"]) {
        let header = header.map(|_| duplicate_header.as_str());
        let ticket = signed(&seed, &kid, header, &body)?;
        let ok = verify_ticket(&ticket, &jwks, issuer, audience, now).is_ok();
        ticket_cases.push(json!({"name": name, "ticket": ticket, "valid": ok}));
    }
    let return_cases: Vec<Value> = return_urls()
        .into_iter()
        .flat_map(|url| {
            [false, true].map(|dev| {
                let ok = validate_return_url(&url, ORIGIN, SCHEME, dev).is_ok();
                json!({"url": url, "allowDevScheme": dev, "valid": ok})
            })
        })
        .collect();
    let document = json!({
        "schemaVersion": 1,
        "seed": SEED,
        "tickets": {"issuer": issuer, "audience": audience, "now": now, "jwks": jwks_value, "cases": ticket_cases},
        "returnUrls": {"origin": ORIGIN, "scheme": SCHEME, "cases": return_cases},
    });
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(OUTPUT);
    fs::create_dir_all(path.parent().ok_or("parent")?)?;
    fs::write(&path, serde_json::to_string_pretty(&document)? + "\n")?;
    Ok(())
}

fn hex(value: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    (0..value.len())
        .step_by(2)
        .map(|i| Ok(u8::from_str_radix(value.get(i..i + 2).ok_or("hex")?, 16)?))
        .collect()
}
