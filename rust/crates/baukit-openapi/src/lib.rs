//! Small, product-neutral helpers for consistent OpenAPI documents.
//!
//! Products continue to own paths, operations, and endpoint schemas. This crate only applies
//! Baukit's document conventions, provides the shared error envelope schema, offers opt-in
//! bearer authentication metadata, and manages a deterministic committed schema.
//!
//! # Example
//!
//! ```rust
//! use baukit_openapi::{ErrorEnvelope, OpenApiMetadata, serialize_schema};
//! use utoipa::openapi::Server;
//!
//! #[derive(utoipa::OpenApi)]
//! #[openapi(components(schemas(ErrorEnvelope)))]
//! struct ApiDoc;
//!
//! let mut document = <ApiDoc as utoipa::OpenApi>::openapi();
//! OpenApiMetadata::new("Orders API", "1.2.3", "The Orders service API")
//!     .servers([Server::new("https://api.example.com")])
//!     .apply_to(&mut document);
//!
//! let json = serialize_schema(&document)?;
//! assert!(json.ends_with('\n'));
//! # Ok::<(), baukit_openapi::SchemaError>(())
//! ```

#![deny(missing_docs)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::openapi::{Components, OpenApi, Server};

/// The component name used for Baukit's standard HTTP bearer JWT security scheme.
pub const BEARER_AUTH_SCHEME: &str = "bearerAuth";

/// Baukit document metadata and optional security conventions.
///
/// New metadata uses `/` as its server, allowing the same schema to describe the service at any
/// deployment origin. Use [`OpenApiMetadata::servers`] when a product has explicit server URLs.
/// Applying metadata preserves product-owned paths, schemas, contact information, license, and
/// existing security schemes. Call [`OpenApiMetadata::bearer_auth`] to add
/// Baukit's standard bearer JWT component; unauthenticated APIs omit it by default.
#[derive(Clone)]
pub struct OpenApiMetadata {
    title: String,
    version: String,
    description: String,
    servers: Vec<Server>,
    bearer_auth: bool,
}

impl OpenApiMetadata {
    /// Creates metadata from a service's display title, version, and safe public description.
    pub fn new<T, V, D>(title: T, version: V, description: D) -> Self
    where
        T: Into<String>,
        V: Into<String>,
        D: Into<String>,
    {
        Self {
            title: title.into(),
            version: version.into(),
            description: description.into(),
            servers: vec![Server::new("/")],
            bearer_auth: false,
        }
    }

    /// Replaces the default relative server with the supplied server entries.
    ///
    /// Passing an empty iterator omits the explicit server list and restores OpenAPI's implicit
    /// `/` server behavior.
    #[must_use]
    pub fn servers<I>(mut self, servers: I) -> Self
    where
        I: IntoIterator<Item = Server>,
    {
        self.servers = servers.into_iter().collect();
        self
    }

    /// Adds Baukit's standard HTTP bearer JWT security-scheme component.
    ///
    /// This only registers the reusable component named [`BEARER_AUTH_SCHEME`];
    /// products still decide which operations or documents require it.
    #[must_use]
    pub const fn bearer_auth(mut self) -> Self {
        self.bearer_auth = true;
        self
    }

    /// Applies metadata, the server list, and any opted-in conventions to a document.
    pub fn apply_to(&self, openapi: &mut OpenApi) {
        openapi.info.title.clone_from(&self.title);
        openapi.info.version.clone_from(&self.version);
        openapi.info.description = Some(self.description.clone());
        openapi.servers = (!self.servers.is_empty()).then(|| self.servers.clone());

        if self.bearer_auth {
            let components = openapi.components.get_or_insert_with(Components::new);
            let bearer = HttpBuilder::new()
                .scheme(HttpAuthScheme::Bearer)
                .bearer_format("JWT")
                .description(Some("OIDC access token supplied as an HTTP bearer JWT."))
                .build();
            components.add_security_scheme(BEARER_AUTH_SCHEME, SecurityScheme::Http(bearer));
        }
    }
}

/// The shared public error response envelope.
///
/// This type deliberately has no HTTP-framework dependency so runtime crates can serialize and
/// reuse it directly.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct ErrorEnvelope {
    /// The structured public error.
    pub error: ErrorBody,
}

/// The error body nested inside [`ErrorEnvelope`].
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct ErrorBody {
    /// Stable, machine-readable error code.
    pub code: String,
    /// Safe user-facing or intentionally generic message.
    pub message: String,
    /// Request identifier shared with logs and support tooling.
    pub request_id: String,
    /// Structured product- or validation-specific error details.
    pub details: BTreeMap<String, Value>,
}

/// An API timestamp with one JSON and OpenAPI representation: an RFC 3339
/// string with OpenAPI `date-time` format.
///
/// Use this at DTO boundaries around a domain [`time::OffsetDateTime`]. The
/// wrapper prevents `time`'s default non-human-readable nine-element tuple from
/// leaking into JSON while keeping conversion to and from the domain type
/// explicit and lossless.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema,
)]
#[schema(value_type = String, format = DateTime)]
pub struct Rfc3339DateTime(#[serde(with = "time::serde::rfc3339")] pub time::OffsetDateTime);

impl Rfc3339DateTime {
    /// Wraps a domain timestamp for use in an API DTO.
    pub const fn new(value: time::OffsetDateTime) -> Self {
        Self(value)
    }

    /// Returns the wrapped domain timestamp.
    pub const fn into_inner(self) -> time::OffsetDateTime {
        self.0
    }
}

impl From<time::OffsetDateTime> for Rfc3339DateTime {
    fn from(value: time::OffsetDateTime) -> Self {
        Self::new(value)
    }
}

impl From<Rfc3339DateTime> for time::OffsetDateTime {
    fn from(value: Rfc3339DateTime) -> Self {
        value.into_inner()
    }
}

impl fmt::Display for Rfc3339DateTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rendered = self
            .0
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|_| fmt::Error)?;
        formatter.write_str(&rendered)
    }
}

impl std::str::FromStr for Rfc3339DateTime {
    type Err = time::error::Parse;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).map(Self)
    }
}

/// A fully typed success-response envelope.
///
/// Both `data` and `meta` remain concrete generic schema arguments. Products
/// should define DTOs for them rather than substituting [`serde_json::Value`],
/// so generated clients retain the complete response contract. A paginated
/// response uses `Vec<ItemDto>` as `T` and a product-owned pagination metadata
/// DTO as `M`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct ResponseEnvelope<T, M> {
    /// The typed response payload.
    pub data: T,
    /// Typed response metadata, such as request ID and pagination cursor.
    pub meta: M,
}

impl<T, M> ResponseEnvelope<T, M> {
    /// Creates a fully typed response envelope.
    pub const fn new(data: T, meta: M) -> Self {
        Self { data, meta }
    }

    /// Maps the payload without weakening the metadata type.
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> ResponseEnvelope<U, M> {
        ResponseEnvelope {
            data: map(self.data),
            meta: self.meta,
        }
    }
}

/// An error produced while serializing, writing, or comparing an OpenAPI schema.
///
/// Its display text includes the affected path and, for drift, the first differing line together
/// with the CI remediation instruction.
#[derive(Debug)]
pub struct SchemaError {
    kind: SchemaErrorKind,
}

#[derive(Debug)]
enum SchemaErrorKind {
    Serialize(serde_json::Error),
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Drift {
        path: PathBuf,
        line: usize,
        committed: Option<String>,
        generated: Option<String>,
        committed_line_count: usize,
        generated_line_count: usize,
    },
}

impl SchemaError {
    fn io(action: &'static str, path: &Path, source: io::Error) -> Self {
        Self {
            kind: SchemaErrorKind::Io {
                action,
                path: path.to_owned(),
                source,
            },
        }
    }

    fn drift(path: &Path, committed: &str, generated: &str) -> Self {
        let committed_lines: Vec<_> = committed.split('\n').collect();
        let generated_lines: Vec<_> = generated.split('\n').collect();
        let difference = committed_lines
            .iter()
            .zip(&generated_lines)
            .position(|(left, right)| left != right)
            .unwrap_or_else(|| committed_lines.len().min(generated_lines.len()));

        Self {
            kind: SchemaErrorKind::Drift {
                path: path.to_owned(),
                line: difference + 1,
                committed: committed_lines.get(difference).map(ToString::to_string),
                generated: generated_lines.get(difference).map(ToString::to_string),
                committed_line_count: committed_lines.len(),
                generated_line_count: generated_lines.len(),
            },
        }
    }

    /// Returns `true` when the committed schema differs from the generated document.
    #[must_use]
    pub fn is_drift(&self) -> bool {
        matches!(self.kind, SchemaErrorKind::Drift { .. })
    }
}

impl fmt::Display for SchemaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            SchemaErrorKind::Serialize(error) => {
                write!(formatter, "could not serialize OpenAPI schema: {error}")
            }
            SchemaErrorKind::Io {
                action,
                path,
                source,
            } => write!(
                formatter,
                "could not {action} OpenAPI schema at {}: {source}",
                path.display()
            ),
            SchemaErrorKind::Drift {
                path,
                line,
                committed,
                generated,
                committed_line_count,
                generated_line_count,
            } => write!(
                formatter,
                "schema drift detected for {} (first difference at line {line}; committed: {}; generated: {}; line counts: {committed_line_count} committed, {generated_line_count} generated)\n\
                 - committed: {}\n\
                 + generated: {}\n\
                 run the openapi binary and commit the generated schema",
                path.display(),
                line_value(committed.as_deref()),
                line_value(generated.as_deref()),
                line_value(committed.as_deref()),
                line_value(generated.as_deref()),
            ),
        }
    }
}

impl StdError for SchemaError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match &self.kind {
            SchemaErrorKind::Serialize(error) => Some(error),
            SchemaErrorKind::Io { source, .. } => Some(source),
            SchemaErrorKind::Drift { .. } => None,
        }
    }
}

impl From<serde_json::Error> for SchemaError {
    fn from(error: serde_json::Error) -> Self {
        Self {
            kind: SchemaErrorKind::Serialize(error),
        }
    }
}

fn line_value(line: Option<&str>) -> String {
    line.map_or_else(|| "<end of file>".to_owned(), |line| format!("{line:?}"))
}

/// Serializes an OpenAPI document as deterministically ordered pretty JSON with a trailing newline.
///
/// Object keys are recursively sorted after Utoipa serialization. This also stabilizes extension
/// maps whose backing collection can otherwise have nondeterministic iteration order.
pub fn serialize_schema(openapi: &OpenApi) -> Result<String, SchemaError> {
    let mut value = serde_json::to_value(openapi)?;
    sort_json_objects(&mut value);
    let mut json = serde_json::to_string_pretty(&value)?;
    json.push('\n');
    Ok(json)
}

fn sort_json_objects(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(sort_json_objects),
        Value::Object(map) => {
            let mut entries: Vec<_> = std::mem::take(map).into_iter().collect();
            for (_, child) in &mut entries {
                sort_json_objects(child);
            }
            entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
            map.extend(entries);
        }
        _ => {}
    }
}

/// Writes a deterministic schema atomically, creating parent directories when necessary.
///
/// The temporary file is created beside the destination so the final rename stays on the same
/// filesystem.
pub fn write_schema(openapi: &OpenApi, path: impl AsRef<Path>) -> Result<(), SchemaError> {
    let path = path.as_ref();
    let json = serialize_schema(openapi)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| SchemaError::io("create parent directories for", path, error))?;

    let (mut temporary, temporary_path) = create_temporary_file(path, parent)?;
    let write_result = (|| {
        temporary
            .write_all(json.as_bytes())
            .map_err(|error| SchemaError::io("write temporary", &temporary_path, error))?;
        temporary
            .sync_all()
            .map_err(|error| SchemaError::io("sync temporary", &temporary_path, error))?;
        drop(temporary);
        fs::rename(&temporary_path, path).map_err(|error| SchemaError::io("replace", path, error))
    })();

    if write_result.is_err() {
        let _ignored = fs::remove_file(&temporary_path);
    }
    write_result
}

static TEMPORARY_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn create_temporary_file(path: &Path, parent: &Path) -> Result<(File, PathBuf), SchemaError> {
    let file_name = path.file_name().ok_or_else(|| {
        SchemaError::io(
            "resolve destination for",
            path,
            io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"),
        )
    })?;

    for _attempt in 0..100 {
        let sequence = TEMPORARY_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary_path = parent.join(format!(
            ".{}.{}.{}.tmp",
            file_name.to_string_lossy(),
            std::process::id(),
            sequence
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((file, temporary_path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(SchemaError::io("create temporary", &temporary_path, error));
            }
        }
    }

    Err(SchemaError::io(
        "create temporary",
        path,
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique temporary file name",
        ),
    ))
}

/// Checks that a committed schema exactly matches deterministic serialization of the document.
///
/// A missing committed file is reported as schema drift, while other read failures are reported as
/// I/O errors.
pub fn check_no_drift(
    openapi: &OpenApi,
    committed_path: impl AsRef<Path>,
) -> Result<(), SchemaError> {
    let committed_path = committed_path.as_ref();
    let generated = serialize_schema(openapi)?;
    let committed = match fs::read_to_string(committed_path) {
        Ok(committed) => committed,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(SchemaError::io("read committed", committed_path, error)),
    };

    if committed == generated {
        Ok(())
    } else {
        Err(SchemaError::drift(committed_path, &committed, &generated))
    }
}

/// Asserts that a committed schema exactly matches deterministic serialization of the document.
///
/// # Panics
///
/// Panics with a drift summary or serialization/I/O error. Use [`check_no_drift`] when the caller
/// should handle the error.
#[track_caller]
pub fn assert_no_drift(openapi: &OpenApi, committed_path: impl AsRef<Path>) {
    if let Err(error) = check_no_drift(openapi, committed_path) {
        panic!("{error}");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};
    use tempfile::tempdir;
    use utoipa::openapi::extensions::Extensions;
    use utoipa::openapi::security::{HttpAuthScheme, SecurityScheme};
    use utoipa::openapi::{Info, OpenApi, Paths, Server};
    use utoipa::{PartialSchema, ToSchema};

    use super::{
        BEARER_AUTH_SCHEME, ErrorBody, ErrorEnvelope, OpenApiMetadata, ResponseEnvelope,
        Rfc3339DateTime, assert_no_drift, check_no_drift, serialize_schema, write_schema,
    };

    fn document() -> OpenApi {
        OpenApi::new(Info::new("Unconfigured", "0.0.0"), Paths::new())
    }

    #[test]
    fn metadata_applies_identity_servers_and_opted_in_bearer_scheme() {
        let mut openapi = document();
        openapi.info.contact = Some(
            utoipa::openapi::ContactBuilder::new()
                .name(Some("Support"))
                .build(),
        );
        OpenApiMetadata::new("Inventory API", "2.4.0", "Inventory operations")
            .servers([
                Server::new("https://api.example.com"),
                Server::new("http://localhost:3000"),
            ])
            .bearer_auth()
            .apply_to(&mut openapi);

        assert_eq!(openapi.info.title, "Inventory API");
        assert_eq!(openapi.info.version, "2.4.0");
        assert_eq!(
            openapi.info.description.as_deref(),
            Some("Inventory operations")
        );
        assert!(openapi.info.contact.is_some());
        let servers = openapi.servers.as_deref();
        assert_eq!(servers.map(|items| items.len()), Some(2));
        assert_eq!(
            servers
                .and_then(|items| items.first())
                .map(|server| server.url.as_str()),
            Some("https://api.example.com")
        );

        let scheme = openapi
            .components
            .as_ref()
            .and_then(|components| components.security_schemes.get(BEARER_AUTH_SCHEME));
        match scheme {
            Some(SecurityScheme::Http(http)) => {
                assert_eq!(http.bearer_format.as_deref(), Some("JWT"));
                assert!(matches!(&http.scheme, HttpAuthScheme::Bearer));
            }
            _ => panic!("expected the standard HTTP bearer scheme"),
        }
        assert!(openapi.security.is_none());
    }

    #[test]
    fn metadata_defaults_to_relative_server() {
        let mut openapi = document();
        OpenApiMetadata::new("Inventory API", "2.4.0", "Inventory operations")
            .apply_to(&mut openapi);
        assert_eq!(
            openapi
                .servers
                .as_ref()
                .and_then(|servers| servers.first())
                .map(|server| server.url.as_str()),
            Some("/")
        );
        assert!(openapi.components.as_ref().is_none_or(|components| {
            !components.security_schemes.contains_key(BEARER_AUTH_SCHEME)
        }));
    }

    #[test]
    fn error_envelope_serializes_to_the_standard_shape() -> Result<(), Box<dyn std::error::Error>> {
        let envelope = ErrorEnvelope {
            error: ErrorBody {
                code: "validation_failed".to_owned(),
                message: "The request is invalid".to_owned(),
                request_id: "req-123".to_owned(),
                details: BTreeMap::new(),
            },
        };

        assert_eq!(
            serde_json::to_value(envelope)?,
            json!({
                "error": {
                    "code": "validation_failed",
                    "message": "The request is invalid",
                    "request_id": "req-123",
                    "details": {}
                }
            })
        );
        Ok(())
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
    struct TimestampDto {
        observed_at: Rfc3339DateTime,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
    struct TestMeta {
        request_id: String,
    }

    #[test]
    fn api_datetime_uses_rfc3339_json_and_date_time_schema()
    -> Result<(), Box<dyn std::error::Error>> {
        let timestamp = "2026-08-09T12:34:56.123456789+02:00".parse::<Rfc3339DateTime>()?;
        let dto = TimestampDto {
            observed_at: timestamp,
        };

        let json = serde_json::to_value(&dto)?;
        assert_eq!(json["observed_at"], "2026-08-09T12:34:56.123456789+02:00");
        assert_eq!(serde_json::from_value::<TimestampDto>(json)?, dto);

        let schema = serde_json::to_value(Rfc3339DateTime::schema())?;
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["format"], "date-time");
        Ok(())
    }

    #[test]
    fn typed_response_envelope_keeps_concrete_payload_and_metadata_schemas()
    -> Result<(), Box<dyn std::error::Error>> {
        let schema = serde_json::to_value(
            <ResponseEnvelope<TimestampDto, TestMeta> as PartialSchema>::schema(),
        )?;
        let rendered = schema.to_string();

        assert!(rendered.contains("TimestampDto"), "{rendered}");
        assert!(rendered.contains("TestMeta"), "{rendered}");
        assert!(!rendered.contains("serde_json.Value"), "{rendered}");
        Ok(())
    }

    #[test]
    fn serialization_is_stable_and_recursively_sorts_maps() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut first = document();
        first.extensions = Some(
            [
                ("x-zeta", json!({"second": 2, "first": 1})),
                ("x-alpha", json!({"z": 2, "a": 1})),
            ]
            .into_iter()
            .collect::<Extensions>(),
        );

        let mut second = document();
        second.extensions = Some(
            [
                ("x-alpha", json!({"a": 1, "z": 2})),
                ("x-zeta", json!({"first": 1, "second": 2})),
            ]
            .into_iter()
            .collect::<Extensions>(),
        );

        let first_once = serialize_schema(&first)?;
        let first_twice = serialize_schema(&first)?;
        assert_eq!(first_once, first_twice);
        assert_eq!(first_once, serialize_schema(&second)?);
        assert!(first_once.ends_with('\n'));

        let parsed: Value = serde_json::from_str(&first_once)?;
        assert_eq!(parsed["x-alpha"], json!({"a": 1, "z": 2}));
        assert!(first_once.find("\"x-alpha\"") < first_once.find("\"x-zeta\""));
        Ok(())
    }

    #[test]
    fn writer_creates_parents_and_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("generated/openapi.json");
        let openapi = document();

        write_schema(&openapi, &path)?;

        assert_eq!(fs::read_to_string(&path)?, serialize_schema(&openapi)?);
        Ok(())
    }

    #[test]
    fn drift_check_accepts_matching_schema_and_rejects_changes_in_both_directions()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("openapi.json");
        let base = document();
        write_schema(&base, &path)?;
        check_no_drift(&base, &path)?;
        assert_no_drift(&base, &path);

        let mut generated_has_more = document();
        generated_has_more.info.description = Some("Additional documentation".to_owned());
        let error = match check_no_drift(&generated_has_more, &path) {
            Err(error) => error,
            Ok(()) => panic!("generated addition should be detected as drift"),
        };
        assert!(error.is_drift());
        assert!(error.to_string().contains("schema drift detected"));
        assert!(
            error
                .to_string()
                .contains("run the openapi binary and commit")
        );

        write_schema(&generated_has_more, &path)?;
        let error = match check_no_drift(&base, &path) {
            Err(error) => error,
            Ok(()) => panic!("generated removal should be detected as drift"),
        };
        assert!(error.is_drift());
        Ok(())
    }

    #[test]
    fn missing_committed_schema_is_drift() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let error = match check_no_drift(&document(), directory.path().join("missing.json")) {
            Err(error) => error,
            Ok(()) => panic!("a missing committed schema should be detected as drift"),
        };
        assert!(error.is_drift());
        Ok(())
    }
}

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;
