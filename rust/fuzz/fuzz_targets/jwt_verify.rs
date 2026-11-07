#![no_main]

use std::{sync::OnceLock, time::Duration};

use baukit_auth::{OidcConfig, OidcVerifier};
use baukit_test::MockOidcServer;
use libfuzzer_sys::fuzz_target;
use tokio::runtime::Runtime;

const AUDIENCE: &str = "api";
const TOKEN_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

struct Harness {
    runtime: Runtime,
    verifier: OidcVerifier,
    valid_token: String,
    _server: MockOidcServer,
}

fn harness() -> &'static Harness {
    static HARNESS: OnceLock<Harness> = OnceLock::new();
    HARNESS.get_or_init(|| {
        let runtime = Runtime::new().expect("tokio runtime");
        let (server, verifier, valid_token) = runtime.block_on(async {
            let server = MockOidcServer::start().await.expect("mock issuer");
            let config = OidcConfig::new(server.issuer(), AUDIENCE).expect("oidc config");
            let verifier =
                OidcVerifier::from_jwks_uri(config, server.jwks_url()).expect("verifier");
            let claims = server
                .claims("fuzz-subject", AUDIENCE, TOKEN_LIFETIME)
                .expect("claims");
            let token = server.mint(&claims).expect("token");
            verifier
                .verify(&token)
                .await
                .expect("minted token verifies");
            (server, verifier, token)
        });
        Harness {
            runtime,
            verifier,
            valid_token,
            _server: server,
        }
    })
}

fuzz_target!(|data: &[u8]| {
    let Ok(token) = std::str::from_utf8(data) else {
        return;
    };
    let harness = harness();
    let mutated = format!("{}{token}", harness.valid_token);
    for candidate in [token, mutated.as_str()] {
        let result = harness.runtime.block_on(harness.verifier.verify(candidate));
        if result.is_ok() {
            assert_eq!(
                candidate, harness.valid_token,
                "a token other than the signed one verified"
            );
        }
    }
});
