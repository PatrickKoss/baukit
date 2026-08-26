# baukit-config

`baukit-config` loads a service's configuration from defaults, an optional file, and environment
variables, validates the whole document before the process starts, and keeps secrets out of logs.
Products define their own fields and invariants; the crate supplies the standard sections and the
loading order.

## Fail at startup, not at 3am

The failure this crate is built to prevent is a service that boots happily with a missing or
nonsensical setting and only discovers it when the first request needs it. `ConfigLoader::load`
deserializes and validates everything up front, so a bad value stops the process while a human is
still watching the deploy.

`Validate` returns every problem at once rather than the first one. Fixing five typos across five
restarts is a worse loop than fixing five typos once.

```rust,no_run
use baukit_config::{BaukitConfig, ConfigLoader, Environment, Validate, ValidationErrors};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ProductConfig {
    catalog_url: Option<String>,
}

impl Validate for ProductConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        Ok(())
    }
}

let config: BaukitConfig<ProductConfig> =
    ConfigLoader::new("myapp", Environment::Local)?.load()?;
# Ok::<(), baukit_config::LoadError>(())
```

Product fields flatten into the top level of the document, next to the standard `http`, `ops`,
`database`, `rate_limit`, `telemetry`, and `shutdown` sections. `database` is `Option`, so a
service with no database omits the section instead of configuring one it never opens.

## Layering

Three layers apply in order, each overriding the last:

1. serde defaults from the config types
2. an optional local file, `config/local.toml` unless you change it
3. environment variables

Environment variables use the application prefix and a double underscore for nesting, so
`MYAPP__HTTP__PORT=8080` sets `http.port`. The prefix is derived from the application name, with
`-` becoming `_`; a name with characters outside ASCII letters, digits, `_`, and `-` is a
`LoadError::InvalidPrefix` rather than a silently mangled prefix.

A `.env` file is read only when the loader's environment is `Environment::Local`. Deployed processes
get their environment from the orchestrator, and a stray `.env` on a production image must not be
able to change what the service does.

Collections use JSON array syntax:

```sh
MYAPP__HTTP__CORS_ALLOWED_ORIGINS='["https://app.example.com"]'
```

`http.cors_allowed_origins` is registered for you. Declare product collection fields with
`ConfigLoader::environment_collection`, otherwise the value arrives as a string.

Scalar environment values keep their exact source strings, including ones that look numeric. That
matters for secrets: an API key of `0123` must not be parsed into `123` on its way to the provider.

Durations in the standard sections deserialize from integer seconds.

## Secrets

Wrap a secret in `Secret<T>`. Its `Debug` and `Display` both print `[redacted]`, and the inner value
is zeroized on drop.

```rust
use baukit_config::Secret;

let token = Secret::new(String::from("s3cr3t"));
assert_eq!(format!("{token:?}"), "[redacted]");
let _exposed = token.expose();
```

Reading the value is spelled `expose` on purpose. `secret.expose()` is greppable in review;
`secret.get()` or a `Deref` would not be. The point is that revealing a secret has to be a visible
choice in the diff rather than something a `{:?}` in a log line does by accident.

## Scope

The crate does not fetch from a secret manager, watch for changes, or reload at runtime. Configuration
is read once at startup and stays fixed for the life of the process, which is what makes it safe to
validate once. Environment and log-format vocabulary comes from `baukit-core` and is re-exported here,
so configuration and telemetry cannot disagree about what `production` means.
