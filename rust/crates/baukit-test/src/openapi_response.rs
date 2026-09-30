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

/// A request a test sent to one documented operation.
///
/// `path` is the documented path template, such as `/items/{id}`, not the
/// request URL. `content_type` is the raw `Content-Type` header value.
#[derive(Clone, Copy, Debug)]
pub struct ObservedRequest<'a> {
    /// HTTP method in any case.
    pub method: &'a str,
    /// Path template as written in the document's `paths` object.
    pub path: &'a str,
    /// Raw `Content-Type` header value, if the request carried one.
    pub content_type: Option<&'a str>,
    /// Request body bytes.
    pub body: &'a [u8],
}

/// Reasons a request or response does not match its OpenAPI operation.
///
/// The location in each message names the operation and either the response
/// status or `request`.
#[derive(Debug, thiserror::Error)]
pub enum OpenApiContractError {
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
        /// Operation and status, or operation and `request`.
        location: String,
        /// The unresolved reference.
        reference: String,
    },
    /// The message has a body but the documented message has no content.
    #[error("{location}: the body is not documented")]
    UndocumentedBody {
        /// Operation and status, or operation and `request`.
        location: String,
    },
    /// The documented response has content, or the documented request body
    /// is required, but the body is empty.
    #[error("{location}: the documented body is missing")]
    MissingBody {
        /// Operation and status, or operation and `request`.
        location: String,
    },
    /// The message has a body without a `Content-Type` header.
    #[error("{location}: the body has no Content-Type")]
    MissingContentType {
        /// Operation and status, or operation and `request`.
        location: String,
    },
    /// The documented message has no entry for the media type.
    #[error("{location}: media type {media_type} is not documented")]
    UndocumentedMediaType {
        /// Operation and status, or operation and `request`.
        location: String,
        /// Observed media type without parameters.
        media_type: String,
    },
    /// A JSON media type carried a body that is not JSON.
    #[error("{location}: the body is not valid JSON: {source}")]
    InvalidJson {
        /// Operation, status or `request`, and media type.
        location: String,
        /// The parse error.
        source: serde_json::Error,
    },
    /// The documented schema does not compile as JSON Schema 2020-12.
    #[error("{location}: the documented schema does not compile: {message}")]
    InvalidSchema {
        /// Operation, status or `request`, and media type.
        location: String,
        /// The compiler's message.
        message: String,
    },
    /// The body violates the documented schema.
    #[error("{location}: the body violates the documented schema:\n- {}", violations.join("\n- "))]
    SchemaViolation {
        /// Operation, status or `request`, and media type.
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
) -> Result<(), OpenApiContractError> {
    let operation = format!("{} {}", response.method.to_uppercase(), response.path);
    let documented = document["paths"][response.path][response.method.to_lowercase()]["responses"]
        .as_object()
        .ok_or_else(|| OpenApiContractError::UndocumentedOperation {
            operation: operation.clone(),
        })?;
    let location = format!("{operation} {}", response.status);
    let documented = select_status(documented, response.status).ok_or(
        OpenApiContractError::UndocumentedStatus {
            operation,
            status: response.status,
        },
    )?;
    let documented = resolve(document, documented, &location)?;
    let message = ObservedBody {
        content_type: response.content_type,
        body: response.body,
    };
    check_body(document, documented, message, true, &location)
}

/// Panics when a response does not match its OpenAPI operation.
///
/// # Panics
///
/// Panics with the [`OpenApiContractError`] message from
/// [`check_response_matches_openapi`].
pub fn assert_response_matches_openapi(document: &Value, response: &ObservedResponse<'_>) {
    if let Err(error) = check_response_matches_openapi(document, response) {
        panic!("{error}");
    }
}

/// Checks a request body against the operation it calls in an OpenAPI 3.1 document.
///
/// The operation's `requestBody` may be a local `$ref`. An empty body passes
/// unless the request body is `required`. The media type matches exactly,
/// then `type/*`, then `*/*`. Bodies of `application/json` and `+json` media
/// types are validated as JSON Schema 2020-12 against the document's
/// `components`, including `format`. Bodies of other media types are not
/// validated. Parameters and headers are not checked.
///
/// # Errors
///
/// Returns the first mismatch, or every schema violation in the body.
pub fn check_request_matches_openapi(
    document: &Value,
    request: &ObservedRequest<'_>,
) -> Result<(), OpenApiContractError> {
    let operation = format!("{} {}", request.method.to_uppercase(), request.path);
    let documented = document["paths"][request.path][request.method.to_lowercase()]
        .as_object()
        .ok_or_else(|| OpenApiContractError::UndocumentedOperation {
            operation: operation.clone(),
        })?;
    let location = format!("{operation} request");
    let documented = match documented.get("requestBody") {
        Some(request_body) => resolve(document, request_body, &location)?,
        None => &Value::Null,
    };
    let required = documented["required"].as_bool().unwrap_or(false);
    let message = ObservedBody {
        content_type: request.content_type,
        body: request.body,
    };
    check_body(document, documented, message, required, &location)
}

/// Panics when a request body does not match its OpenAPI operation.
///
/// # Panics
///
/// Panics with the [`OpenApiContractError`] message from
/// [`check_request_matches_openapi`].
pub fn assert_request_matches_openapi(document: &Value, request: &ObservedRequest<'_>) {
    if let Err(error) = check_request_matches_openapi(document, request) {
        panic!("{error}");
    }
}

#[derive(Clone, Copy)]
struct ObservedBody<'a> {
    content_type: Option<&'a str>,
    body: &'a [u8],
}

fn check_body(
    document: &Value,
    documented: &Value,
    message: ObservedBody<'_>,
    body_required: bool,
    location: &str,
) -> Result<(), OpenApiContractError> {
    let location = location.to_owned();
    let content = documented["content"]
        .as_object()
        .filter(|content| !content.is_empty());
    match (content, message.body.is_empty()) {
        (None, true) => Ok(()),
        (None, false) => Err(OpenApiContractError::UndocumentedBody { location }),
        (Some(_), true) if body_required => Err(OpenApiContractError::MissingBody { location }),
        (Some(_), true) => Ok(()),
        (Some(content), false) => check_content(document, content, message, &location),
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
) -> Result<&'a Value, OpenApiContractError> {
    let mut hops = 0;
    while let Some(reference) = value["$ref"].as_str() {
        let unresolved = || OpenApiContractError::UnresolvedReference {
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
    ["responses", "requestBodies"]
        .iter()
        .filter_map(|section| document["components"][section].as_object())
        .map(serde_json::Map::len)
        .sum::<usize>()
        + 1
}

fn check_content(
    document: &Value,
    content: &serde_json::Map<String, Value>,
    message: ObservedBody<'_>,
    location: &str,
) -> Result<(), OpenApiContractError> {
    let media_type = message
        .content_type
        .map(essence)
        .filter(|media_type| !media_type.is_empty())
        .ok_or_else(|| OpenApiContractError::MissingContentType {
            location: location.to_owned(),
        })?;
    let media = select_media_type(content, &media_type).ok_or_else(|| {
        OpenApiContractError::UndocumentedMediaType {
            location: location.to_owned(),
            media_type: media_type.clone(),
        }
    })?;
    if !is_json(&media_type) {
        return Ok(());
    }
    let location = format!("{location} {media_type}");
    let body = serde_json::from_slice::<Value>(message.body).map_err(|source| {
        OpenApiContractError::InvalidJson {
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
) -> Result<(), OpenApiContractError> {
    let wrapper = json!({
        "$schema": JSON_SCHEMA_DIALECT,
        "components": document["components"],
        "allOf": [schema],
    });
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&wrapper)
        .map_err(|error| OpenApiContractError::InvalidSchema {
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
    Err(OpenApiContractError::SchemaViolation {
        location,
        violations,
    })
}

fn pointer_label(pointer: &str) -> &str {
    if pointer.is_empty() { "/" } else { pointer }
}
