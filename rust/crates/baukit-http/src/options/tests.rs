use baukit_core::DeploymentEnvironment;

use super::*;

fn rejected(origin: &str, environment: DeploymentEnvironment) -> bool {
    matches!(
        parse_origins([origin], environment),
        Err(HttpOptionsError::InvalidOrigin(_))
    )
}

#[test]
fn null_and_path_origins_are_rejected_everywhere() {
    for environment in [
        DeploymentEnvironment::Local,
        DeploymentEnvironment::Production,
    ] {
        for origin in [
            "null",
            "https://app.example.com/",
            "https://app.example.com/path",
            "https://app.example.com?x=1",
            "https://app.example.com#f",
            "https://user@app.example.com",
            "https://",
            "ftp://app.example.com",
            "app.example.com",
        ] {
            assert!(rejected(origin, environment), "{origin} in {environment:?}");
        }
    }
}

#[test]
fn plain_http_is_local_only_except_loopback() {
    assert!(rejected(
        "http://app.example.com",
        DeploymentEnvironment::Production
    ));
    assert!(rejected(
        "http://app.example.com",
        DeploymentEnvironment::Staging
    ));
    assert!(
        parse_origins(
            ["http://app.example.com:8080"],
            DeploymentEnvironment::Local
        )
        .is_ok()
    );
    for origin in [
        "http://localhost:5173",
        "http://127.0.0.1:3000",
        "http://[::1]:8080",
    ] {
        assert!(
            parse_origins([origin], DeploymentEnvironment::Production).is_ok(),
            "{origin}"
        );
    }
    assert!(rejected(
        "http://localhost.evil.example",
        DeploymentEnvironment::Production
    ));
}

#[test]
fn https_origins_with_ports_are_accepted() {
    let parsed = parse_origins(
        ["https://app.example.com", "https://app.example.com:8443"],
        DeploymentEnvironment::Production,
    )
    .expect("valid origins");
    assert_eq!(parsed.len(), 2);
}

#[test]
fn builder_allows_loopback_http_but_not_remote_http() {
    assert!(
        HttpOptions::default()
            .with_allowed_origins(["http://localhost:5173"])
            .is_ok()
    );
    assert!(
        HttpOptions::default()
            .with_allowed_origins(["http://app.example.com"])
            .is_err()
    );
}

#[test]
fn from_config_in_allows_http_only_locally() {
    let config = HttpConfig {
        cors_allowed_origins: vec!["http://web.internal:8080".to_owned()],
        ..HttpConfig::default()
    };
    assert!(HttpOptions::from_config_in(&config, DeploymentEnvironment::Local).is_ok());
    assert!(HttpOptions::from_config_in(&config, DeploymentEnvironment::Production).is_err());
    assert!(HttpOptions::from_config(&config).is_err());
}
