use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;

use super::*;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ProductConfig {
    feature_limit: usize,
    labels: Vec<String>,
    json_array_secret: Option<Secret<String>>,
    leading_zero_secret: Option<Secret<String>>,
    exponent_secret: Option<Secret<String>>,
}

impl Validate for ProductConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        if self.feature_limit == 13 {
            Err(ValidationErrors::new(vec![ValidationError::new(
                "feature_limit",
                "must not be unlucky",
            )]))
        } else {
            Ok(())
        }
    }
}

#[test]
fn default_layer_supplies_standard_values() {
    let config = ConfigLoader::new("defaults_test", Environment::Local)
        .expect("valid loader")
        .without_local_file()
        .without_dotenv()
        .load::<ProductConfig>()
        .expect("valid defaults");

    assert_eq!(config.environment, Environment::Local);
    assert_eq!(config.http.port, 8080);
    assert_eq!(config.ops.port, 9090);
    assert!(config.database.is_none());
    assert_eq!(config.shutdown.drain_timeout, Duration::from_secs(30));
    assert_eq!(config.product.feature_limit, 0);
}

#[test]
fn unit_product_uses_standard_configuration_only() {
    let config = ConfigLoader::new("unit_product", Environment::Local)
        .expect("valid loader")
        .without_local_file()
        .without_dotenv()
        .load::<()>()
        .expect("unit product config");

    assert_eq!(config.http.port, 8080);
    assert_eq!(config.product, ());
}

#[test]
fn local_file_overrides_defaults() {
    let directory = TestDirectory::new();
    let file = directory.path().join("local.toml");
    fs::write(
        &file,
        r#"
feature_limit = 7

[http]
port = 8181
request_timeout = 12

[database]
url = "postgres://file-secret@localhost/file"
"#,
    )
    .expect("write local config");

    let config = ConfigLoader::new("file_test", Environment::Local)
        .expect("valid loader")
        .local_file(file)
        .without_dotenv()
        .load::<ProductConfig>()
        .expect("valid file config");

    assert_eq!(config.http.port, 8181);
    assert_eq!(config.http.request_timeout, Duration::from_secs(12));
    assert_eq!(config.product.feature_limit, 7);
    let database = config.database.as_ref().expect("database section");
    assert_eq!(
        database.url.expose(),
        "postgres://file-secret@localhost/file"
    );
    assert_eq!(database.max_connections, 10);
    assert_eq!(database.min_connections, 1);
}

#[test]
fn environment_override_precedence_and_separator_work() {
    run_env_helper(
        "env_override_helper",
        &[
            ("LAYER_APP__HTTP__PORT", "8282"),
            ("LAYER_APP__HTTP__REQUEST_TIMEOUT", "17"),
            ("LAYER_APP__FEATURE_LIMIT", "9"),
        ],
    );
}

#[test]
fn numeric_looking_environment_secrets_remain_literal_strings() {
    run_env_helper(
        "numeric_looking_environment_secrets_helper",
        &[
            ("SECRET_LITERAL_APP__LEADING_ZERO_SECRET", "0123"),
            ("SECRET_LITERAL_APP__EXPONENT_SECRET", "1e5"),
            ("SECRET_LITERAL_APP__FEATURE_LIMIT", "9"),
        ],
    );
}

#[test]
fn collection_valued_environment_fields_accept_json_arrays() {
    run_env_helper(
        "collection_environment_helper",
        &[
            (
                "COLLECTION_APP__HTTP__CORS_ALLOWED_ORIGINS",
                r#"["https://app.example.com","http://127.0.0.1:5173"]"#,
            ),
            ("COLLECTION_APP__LABELS", r#"["alpha","beta"]"#),
            (
                "COLLECTION_APP__JSON_ARRAY_SECRET",
                r#"["this","is","literal"]"#,
            ),
        ],
    );
}

#[test]
fn deployed_environment_never_loads_dotenv() {
    let directory = TestDirectory::new();
    let dotenv = directory.path().join("production.env");
    fs::write(&dotenv, "DOTENV_APP__HTTP__PORT=8383\n").expect("write dotenv");

    let config = ConfigLoader::new("dotenv_app", Environment::Production)
        .expect("valid loader")
        .without_local_file()
        .dotenv_file(dotenv)
        .load::<ProductConfig>()
        .expect("valid production config");

    assert_eq!(config.environment, Environment::Production);
    assert_eq!(config.http.port, 8080);
}

#[test]
fn local_environment_loads_dotenv() {
    run_env_helper("local_dotenv_helper", &[]);
}

#[test]
fn local_dotenv_helper() {
    if std::env::var_os("BAUKIT_CONFIG_ENV_HELPER").is_none() {
        return;
    }

    let directory = TestDirectory::new();
    let dotenv = directory.path().join("local.env");
    fs::write(
        &dotenv,
        "LOCAL_DOTENV_APP__HTTP__PORT=8484\nOTEL_EXPORTER_OTLP_ENDPOINT=http://collector:4318\n",
    )
    .expect("write dotenv");

    let config = ConfigLoader::new("local-dotenv-app", Environment::Local)
        .expect("valid loader")
        .without_local_file()
        .dotenv_file(dotenv)
        .load::<ProductConfig>()
        .expect("valid local dotenv config");

    assert_eq!(config.http.port, 8484);
    assert_eq!(
        config.telemetry.otlp_endpoint.as_deref(),
        Some("http://collector:4318")
    );
}

#[test]
fn env_override_helper() {
    if std::env::var_os("BAUKIT_CONFIG_ENV_HELPER").is_none() {
        return;
    }

    let directory = TestDirectory::new();
    let file = directory.path().join("local.toml");
    fs::write(
        &file,
        r#"
feature_limit = 7

[http]
port = 8181
request_timeout = 12
"#,
    )
    .expect("write local config");

    let loader = ConfigLoader::new("layer-app", Environment::Staging)
        .expect("valid loader")
        .local_file(file);
    assert_eq!(loader.environment_prefix(), "LAYER_APP");
    let config = loader.load::<ProductConfig>().expect("valid env config");

    assert_eq!(config.http.port, 8282);
    assert_eq!(config.http.request_timeout, Duration::from_secs(17));
    assert_eq!(config.product.feature_limit, 9);
    assert_eq!(config.environment, Environment::Staging);
}

#[test]
fn numeric_looking_environment_secrets_helper() {
    if std::env::var_os("BAUKIT_CONFIG_ENV_HELPER").is_none() {
        return;
    }

    let config = ConfigLoader::new("secret-literal-app", Environment::Staging)
        .expect("valid loader")
        .without_local_file()
        .without_dotenv()
        .load::<ProductConfig>()
        .expect("valid env config");

    assert_eq!(
        config
            .product
            .leading_zero_secret
            .as_ref()
            .expect("leading-zero secret")
            .expose(),
        "0123"
    );
    assert_eq!(
        config
            .product
            .exponent_secret
            .as_ref()
            .expect("exponent secret")
            .expose(),
        "1e5"
    );
    assert_eq!(config.product.feature_limit, 9);
}

#[test]
fn collection_environment_helper() {
    if std::env::var_os("BAUKIT_CONFIG_ENV_HELPER").is_none() {
        return;
    }

    let config = ConfigLoader::new("collection-app", Environment::Staging)
        .expect("valid loader")
        .without_local_file()
        .without_dotenv()
        .environment_collection("labels")
        .load::<ProductConfig>()
        .expect("valid collection env config");

    assert_eq!(
        config.http.cors_allowed_origins,
        ["https://app.example.com", "http://127.0.0.1:5173"]
    );
    assert_eq!(config.product.labels, ["alpha", "beta"]);
    assert_eq!(
        config
            .product
            .json_array_secret
            .as_ref()
            .expect("JSON-looking secret")
            .expose(),
        r#"["this","is","literal"]"#
    );
}

#[test]
fn secret_is_redacted_in_debug_display_and_validation_errors() {
    let secret_text = "postgres://admin:very-secret@localhost/app";
    let secret = Secret::new(secret_text.to_owned());
    assert_eq!(format!("{secret:?}"), "[redacted]");
    assert_eq!(secret.to_string(), "[redacted]");

    let config = BaukitConfig::<ProductConfig> {
        database: Some(DatabaseConfig {
            url: Secret::new(secret_text.to_owned()),
            max_connections: 0,
            ..DatabaseConfig::default()
        }),
        ..BaukitConfig::default()
    };
    let error = config
        .validate()
        .expect_err("invalid pool size")
        .to_string();
    assert!(error.contains("database.max_connections: must be non-zero"));
    assert!(!error.contains(secret_text));
    assert!(!format!("{config:?}").contains(secret_text));
}

#[test]
fn validation_aggregates_qualified_errors() {
    let mut config = BaukitConfig::<ProductConfig>::default();
    config.http.port = 0;
    config.ops.port = 0;
    config.database = Some(DatabaseConfig {
        max_connections: 1,
        min_connections: 2,
        ..DatabaseConfig::default()
    });
    config.telemetry.trace_sampling_ratio = 1.5;
    config.shutdown.drain_timeout = Duration::ZERO;
    config.product.feature_limit = 13;

    let errors = config.validate().expect_err("multiple invalid fields");
    let paths: Vec<_> = errors.errors().iter().map(ValidationError::path).collect();
    assert_eq!(
        paths,
        [
            "http.port",
            "ops.port",
            "database.min_connections",
            "telemetry.trace_sampling_ratio",
            "shutdown.drain_timeout",
            "product.feature_limit",
        ]
    );
    let rendered = errors.to_string();
    assert!(rendered.contains("configuration validation failed (6 error(s))"));
    assert!(rendered.contains("http.port: must be non-zero"));
    assert!(rendered.contains("product.feature_limit: must not be unlucky"));
}

#[test]
fn environment_parsing_and_rendering_follow_contract() {
    for (text, expected) in [
        ("local", Environment::Local),
        ("testing", Environment::Testing),
        ("staging", Environment::Staging),
        ("production", Environment::Production),
    ] {
        assert_eq!(text.parse::<Environment>(), Ok(expected));
        assert_eq!(expected.to_string(), text);
    }
    assert_eq!(
        "prod"
            .parse::<Environment>()
            .expect_err("unsupported name")
            .to_string(),
        "unsupported environment `prod`; expected local, testing, staging, or production"
    );
}

#[test]
fn automatic_log_format_depends_on_environment() {
    assert_eq!(
        LogFormat::Auto.resolve(Environment::Local),
        LogFormat::Pretty
    );
    assert_eq!(
        LogFormat::Auto.resolve(Environment::Testing),
        LogFormat::Json
    );
    assert_eq!(
        LogFormat::Auto.resolve(Environment::Staging),
        LogFormat::Json
    );
    assert_eq!(
        LogFormat::Auto.resolve(Environment::Production),
        LogFormat::Json
    );
    assert_eq!(
        LogFormat::Pretty.resolve(Environment::Production),
        LogFormat::Pretty
    );
}

fn run_env_helper(test_name: &str, environment: &[(&str, &str)]) {
    let executable = std::env::current_exe().expect("test executable");
    let mut command = Command::new(executable);
    command
        .arg("--exact")
        .arg(format!("tests::{test_name}"))
        .arg("--nocapture")
        .env("BAUKIT_CONFIG_ENV_HELPER", "1");
    for (key, value) in environment {
        command.env(key, value);
    }
    let output = command.output().expect("run isolated environment test");
    assert!(
        output.status.success(),
        "environment helper failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("baukit-config-{}-{unique}", std::process::id()));
        fs::create_dir(&path).expect("create test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
