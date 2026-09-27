use serde_json::{Value, json};

const JSON_SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";
const LOCAL_REFERENCE_PREFIX: &str = "#/";

/// A response a test received from one documented operation.
///
/// `path` is the documented path template, such as `/items/{id}`, not the
/// request URL. `content_type` is the raw `Content-Type` header value.
#[derive(Clone, Copy, Debug)]
pub struct ObservedResponse<'a> {
    /// HTTP method in any case.
    pub method: &'a str,
    /// Path template as written in the document's `paths` object.
    pub path: &'a str,
    /// Response status code.
    pub status: u16,
    /// Raw `Content-Type` header value, if the response carried one.
    pub content_type: Option<&'a str>,
    /// Response body bytes.
    pub body: &'a [u8],
}

/// Reasons a response does not match its OpenAPI operation.
#[derive(Debug, thiserror::Error)]
pub enum OpenApiResponseError {
    /// The document has no operation for the method and path template.
    #[error("{operation} is not documented")]
    UndocumentedOperation {
        /// Method and path template.
        operation: String,
    },
    /// The operation documents neither the status, its class, nor `default`.
    #[error("{operation} does not document status {status}")]
    UndocumentedStatus {
        /// Method and path template.
        operation: String,
        /// Observed status code.
        status: u16,
    },
    /// A local `$ref` does not resolve inside the document.
    #[error("{location}: reference {reference} does not resolve")]
    UnresolvedReference {
        /// Operation and status.
        location: String,
        /// The unresolved reference.
        reference: String,
    },
    /// The response has a body but the documented response has no content.
    #[error("{location}: the body is not documented")]
    UndocumentedBody {
        /// Operation and status.
        location: String,
    },
    /// The documented response has content but the body is empty.
    #[error("{location}: the documented body is missing")]
    MissingBody {
        /// Operation and status.
        location: String,
    },
    /// The response has a body without a `Content-Type` header.
    #[error("{location}: the body has no Content-Type")]
    MissingContentType {
        /// Operation and status.
        location: String,
    },
    /// The documented response has no entry for the media type.
    #[error("{location}: media type {media_type} is not documented")]
    UndocumentedMediaType {
        /// Operation and status.
        location: String,
        /// Observed media type without parameters.
        media_type: String,
    },
    /// A JSON media type carried a body that is not JSON.
    #[error("{location}: the body is not valid JSON: {source}")]
    InvalidJson {
        /// Operation, status, and media type.
        location: String,
        /// The parse error.
        source: serde_json::Error,
    },
    /// The documented schema does not compile as JSON Schema 2020-12.
    #[error("{location}: the documented schema does not compile: {message}")]
    InvalidSchema {
        /// Operation, status, and media type.
        location: String,
        /// The compiler's message.
        message: String,
    },
    /// The body violates the documented schema.
    #[error("{location}: the body violates the documented schema:\n- {}", violations.join("\n- "))]
    SchemaViolation {
        /// Operation, status, and media type.
        location: String,
        /// One entry per violation, prefixed with the JSON pointer of the value.
        violations: Vec<String>,
    },
}

/// Checks a response against the operation it answers in an OpenAPI 3.1 document.
///
/// The status matches an exact code, then its class such as `4XX`, then
/// `default`. The media type matches exactly, then `type/*`, then `*/*`.
/// Bodies of `application/json` and `+json` media types are validated as JSON
/// Schema 2020-12 against the document's `components`, including `format`.
/// Bodies of other media types are not validated.
///
/// # Errors
///
/// Returns the first mismatch, or every schema violation in the body.
pub fn check_response_matches_openapi(
    document: &Value,
    response: &ObservedResponse<'_>,
) -> Result<(), OpenApiResponseError> {
    let operation = format!("{} {}", response.method.to_uppercase(), response.path);
    let documented = document["paths"][response.path][response.method.to_lowercase()]["responses"]
        .as_object()
        .ok_or_else(|| OpenApiResponseError::UndocumentedOperation {
            operation: operation.clone(),
        })?;
    let location = format!("{operation} {}", response.status);
    let documented = select_status(documented, response.status).ok_or(
        OpenApiResponseError::UndocumentedStatus {
            operation,
            status: response.status,
        },
    )?;
    let documented = resolve(document, documented, &location)?;
    let content = documented["content"]
        .as_object()
        .filter(|content| !content.is_empty());
    match (content, response.body.is_empty()) {
        (None, true) => Ok(()),
        (None, false) => Err(OpenApiResponseError::UndocumentedBody { location }),
        (Some(_), true) => Err(OpenApiResponseError::MissingBody { location }),
        (Some(content), false) => check_content(document, content, response, &location),
    }
}

/// Panics when a response does not match its OpenAPI operation.
///
/// # Panics
///
/// Panics with the [`OpenApiResponseError`] message from
/// [`check_response_matches_openapi`].
pub fn assert_response_matches_openapi(document: &Value, response: &ObservedResponse<'_>) {
    if let Err(error) = check_response_matches_openapi(document, response) {
        panic!("{error}");
    }
}

fn select_status(responses: &serde_json::Map<String, Value>, status: u16) -> Option<&Value> {
    let class = format!("{}XX", status / 100);
    responses
        .get(&status.to_string())
        .or_else(|| responses.get(&class))
        .or_else(|| responses.get(&class.to_lowercase()))
        .or_else(|| responses.get("default"))
}

fn resolve<'a>(
    document: &'a Value,
    mut value: &'a Value,
    location: &str,
) -> Result<&'a Value, OpenApiResponseError> {
    let mut hops = 0;
    while let Some(reference) = value["$ref"].as_str() {
        let unresolved = || OpenApiResponseError::UnresolvedReference {
            location: location.to_owned(),
            reference: reference.to_owned(),
        };
        hops += 1;
        if hops > max_reference_hops(document) {
            return Err(unresolved());
        }
        value = reference
            .strip_prefix(LOCAL_REFERENCE_PREFIX)
            .and_then(|pointer| document.pointer(&format!("/{pointer}")))
            .ok_or_else(unresolved)?;
    }
    Ok(value)
}

fn max_reference_hops(document: &Value) -> usize {
    document["components"]["responses"]
        .as_object()
        .map_or(0, serde_json::Map::len)
        + 1
}

fn check_content(
    document: &Value,
    content: &serde_json::Map<String, Value>,
    response: &ObservedResponse<'_>,
    location: &str,
) -> Result<(), OpenApiResponseError> {
    let media_type = response
        .content_type
        .map(essence)
        .filter(|media_type| !media_type.is_empty())
        .ok_or_else(|| OpenApiResponseError::MissingContentType {
            location: location.to_owned(),
        })?;
    let media = select_media_type(content, &media_type).ok_or_else(|| {
        OpenApiResponseError::UndocumentedMediaType {
            location: location.to_owned(),
            media_type: media_type.clone(),
        }
    })?;
    if !is_json(&media_type) {
        return Ok(());
    }
    let location = format!("{location} {media_type}");
    let body = serde_json::from_slice::<Value>(response.body).map_err(|source| {
        OpenApiResponseError::InvalidJson {
            location: location.clone(),
            source,
        }
    })?;
    match media.get("schema") {
        Some(schema) => check_schema(document, schema, &body, location),
        None => Ok(()),
    }
}

fn essence(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn select_media_type<'a>(
    content: &'a serde_json::Map<String, Value>,
    media_type: &str,
) -> Option<&'a Value> {
    let exact = content
        .iter()
        .find(|(documented, _)| documented.eq_ignore_ascii_case(media_type));
    let wildcard = media_type
        .split_once('/')
        .and_then(|(kind, _)| content.get(&format!("{kind}/*")));
    exact
        .map(|(_, media)| media)
        .or(wildcard)
        .or_else(|| content.get("*/*"))
}

fn is_json(media_type: &str) -> bool {
    media_type == "application/json" || media_type.ends_with("+json")
}

fn check_schema(
    document: &Value,
    schema: &Value,
    body: &Value,
    location: String,
) -> Result<(), OpenApiResponseError> {
    let wrapper = json!({
        "$schema": JSON_SCHEMA_DIALECT,
        "components": document["components"],
        "allOf": [schema],
    });
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&wrapper)
        .map_err(|error| OpenApiResponseError::InvalidSchema {
            location: location.clone(),
            message: error.to_string(),
        })?;
    let violations = validator
        .iter_errors(body)
        .map(|error| {
            format!(
                "{}: {error}",
                pointer_label(&error.instance_path().to_string())
            )
        })
        .collect::<Vec<_>>();
    if violations.is_empty() {
        return Ok(());
    }
    Err(OpenApiResponseError::SchemaViolation {
        location,
        violations,
    })
}

fn pointer_label(pointer: &str) -> &str {
    if pointer.is_empty() { "/" } else { pointer }
}
