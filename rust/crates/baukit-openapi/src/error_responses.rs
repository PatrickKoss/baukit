use utoipa::ToSchema;
use utoipa::openapi::header::{Header, HeaderBuilder};
use utoipa::openapi::path::{HttpMethod, Operation, ParameterIn, PathItem};
use utoipa::openapi::response::{Response, ResponseBuilder};
use utoipa::openapi::security::SecurityRequirement;
use utoipa::openapi::{ContentBuilder, ObjectBuilder, OpenApi, Ref, RefOr, Type};

use crate::ErrorEnvelope;

/// The response header that carries the request identifier.
pub const REQUEST_ID_HEADER: &str = "X-Request-Id";

/// The response header that tells a client how many seconds to wait before retrying.
pub const RETRY_AFTER_HEADER: &str = "Retry-After";

/// The response header that carries an authentication challenge.
pub const WWW_AUTHENTICATE_HEADER: &str = "WWW-Authenticate";

const IDEMPOTENCY_KEY_HEADER: &str = "Idempotency-Key";
const JSON_CONTENT_TYPE: &str = "application/json";
const PATH_PARAMETER_START: char = '{';
const STATUS_UNAUTHORIZED: u16 = 401;
const STATUS_TOO_MANY_REQUESTS: u16 = 429;

/// A property of an operation that decides whether a rule applies to it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationCondition {
    /// Every operation.
    Always,
    /// The operation's own `security`, or the document's when the operation has none, contains a
    /// requirement that names a scheme. An operation that also accepts anonymous calls still
    /// counts, because a bad credential is rejected.
    Secured,
    /// The operation declares a request body.
    HasRequestBody,
    /// The path template has a parameter, such as `/items/{id}`.
    HasPathParameter,
    /// The method is not safe in the RFC 9110 sense: `POST`, `PUT`, `PATCH`, or `DELETE`.
    UnsafeMethod,
    /// The operation declares an `Idempotency-Key` header parameter.
    HasIdempotencyKey,
}

/// Which responses of a matching operation a header rule documents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseSelector {
    /// Every inline response, including success responses.
    Every,
    /// The inline response with this status code.
    Status(u16),
}

#[derive(Clone, Debug)]
struct StatusRule {
    condition: OperationCondition,
    status: String,
    description: String,
}

#[derive(Clone)]
struct HeaderRule {
    condition: OperationCondition,
    responses: ResponseSelector,
    name: String,
    header: Header,
}

/// Error responses and response headers to document on every matching operation, as data.
///
/// A status rule adds a response with the shared [`ErrorEnvelope`] schema and the product's
/// description when the operation does not document that status yet. A header rule adds a header
/// to the selected inline responses when they do not have a header of that name, compared without
/// regard to case. Responses given as a `$ref` are left unchanged, because they are shared
/// components. Applying the same rules twice changes nothing.
#[derive(Clone, Default)]
pub struct ErrorResponseRules {
    statuses: Vec<StatusRule>,
    headers: Vec<HeaderRule>,
}

impl ErrorResponseRules {
    /// Creates an empty rule set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an error response with this status and description to operations that match.
    #[must_use]
    pub fn status(
        mut self,
        condition: OperationCondition,
        status: u16,
        description: impl Into<String>,
    ) -> Self {
        self.statuses.push(StatusRule {
            condition,
            status: status.to_string(),
            description: description.into(),
        });
        self
    }

    /// Adds a response header to the selected responses of operations that match.
    #[must_use]
    pub fn header(
        mut self,
        condition: OperationCondition,
        responses: ResponseSelector,
        name: impl Into<String>,
        header: Header,
    ) -> Self {
        self.headers.push(HeaderRule {
            condition,
            responses,
            name: name.into(),
            header,
        });
        self
    }

    /// Adds the headers that `baukit-http` and `baukit-auth` send: `X-Request-Id` on every
    /// response, `Retry-After` on 429, and `WWW-Authenticate` on 401.
    #[must_use]
    pub fn standard_headers(self) -> Self {
        self.header(
            OperationCondition::Always,
            ResponseSelector::Every,
            REQUEST_ID_HEADER,
            request_id_header(),
        )
        .header(
            OperationCondition::Always,
            ResponseSelector::Status(STATUS_TOO_MANY_REQUESTS),
            RETRY_AFTER_HEADER,
            retry_after_header(),
        )
        .header(
            OperationCondition::Always,
            ResponseSelector::Status(STATUS_UNAUTHORIZED),
            WWW_AUTHENTICATE_HEADER,
            www_authenticate_header(),
        )
    }

    /// Applies the rules to every operation in the document.
    pub fn apply(&self, document: &mut OpenApi) {
        self.apply_where(document, |_, _| true);
    }

    /// Applies the rules to the operations whose path template and method pass `include`.
    ///
    /// Use it for statuses only some routes return, such as a 412 on conditional writes, or to
    /// skip public documents such as the OpenAPI route itself.
    pub fn apply_where(&self, document: &mut OpenApi, include: impl Fn(&str, &HttpMethod) -> bool) {
        let document_security = document.security.clone();
        for (path, item) in &mut document.paths.paths {
            for (method, operation) in operations_mut(item) {
                let Some(operation) = operation else {
                    continue;
                };
                if !include(path, &method) {
                    continue;
                }
                let facts = OperationFacts::new(path, &method, operation, &document_security);
                self.apply_to_operation(&facts, operation);
            }
        }
    }

    fn apply_to_operation(&self, facts: &OperationFacts, operation: &mut Operation) {
        for rule in &self.statuses {
            if facts.matches(rule.condition) {
                insert_error_response(operation, &rule.status, &rule.description);
            }
        }
        for rule in &self.headers {
            if facts.matches(rule.condition) {
                insert_header(operation, rule);
            }
        }
    }
}

struct OperationFacts {
    secured: bool,
    has_request_body: bool,
    has_path_parameter: bool,
    unsafe_method: bool,
    has_idempotency_key: bool,
}

impl OperationFacts {
    fn new(
        path: &str,
        method: &HttpMethod,
        operation: &Operation,
        document_security: &Option<Vec<SecurityRequirement>>,
    ) -> Self {
        let security = operation.security.as_ref().or(document_security.as_ref());
        Self {
            secured: security.is_some_and(|requirements| requirements.iter().any(names_a_scheme)),
            has_request_body: operation.request_body.is_some(),
            has_path_parameter: path.contains(PATH_PARAMETER_START),
            unsafe_method: matches!(
                method,
                HttpMethod::Post | HttpMethod::Put | HttpMethod::Patch | HttpMethod::Delete
            ),
            has_idempotency_key: operation.parameters.iter().flatten().any(|parameter| {
                matches!(parameter, RefOr::T(parameter) if parameter.parameter_in == ParameterIn::Header
                    && parameter.name.eq_ignore_ascii_case(IDEMPOTENCY_KEY_HEADER))
            }),
        }
    }

    const fn matches(&self, condition: OperationCondition) -> bool {
        match condition {
            OperationCondition::Always => true,
            OperationCondition::Secured => self.secured,
            OperationCondition::HasRequestBody => self.has_request_body,
            OperationCondition::HasPathParameter => self.has_path_parameter,
            OperationCondition::UnsafeMethod => self.unsafe_method,
            OperationCondition::HasIdempotencyKey => self.has_idempotency_key,
        }
    }
}

fn names_a_scheme(requirement: &SecurityRequirement) -> bool {
    *requirement != SecurityRequirement::default()
}

fn operations_mut(item: &mut PathItem) -> [(HttpMethod, &mut Option<Operation>); 8] {
    [
        (HttpMethod::Get, &mut item.get),
        (HttpMethod::Put, &mut item.put),
        (HttpMethod::Post, &mut item.post),
        (HttpMethod::Delete, &mut item.delete),
        (HttpMethod::Options, &mut item.options),
        (HttpMethod::Head, &mut item.head),
        (HttpMethod::Patch, &mut item.patch),
        (HttpMethod::Trace, &mut item.trace),
    ]
}

fn insert_header(operation: &mut Operation, rule: &HeaderRule) {
    for (status, response) in &mut operation.responses.responses {
        let RefOr::T(response) = response else {
            continue;
        };
        if !selects(rule.responses, status) {
            continue;
        }
        let present = response
            .headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case(&rule.name));
        if !present {
            response
                .headers
                .insert(rule.name.clone(), RefOr::T(rule.header.clone()));
        }
    }
}

fn selects(selector: ResponseSelector, status: &str) -> bool {
    match selector {
        ResponseSelector::Every => true,
        ResponseSelector::Status(selected) => status.parse::<u16>() == Ok(selected),
    }
}

/// Adds an error response with the shared envelope unless the operation documents the status.
pub(crate) fn insert_error_response(operation: &mut Operation, status: &str, description: &str) {
    operation
        .responses
        .responses
        .entry(status.to_owned())
        .or_insert_with(|| RefOr::T(error_response(description)));
}

fn error_response(description: &str) -> Response {
    let schema = Ref::from_schema_name(<ErrorEnvelope as ToSchema>::name());
    ResponseBuilder::new()
        .description(description)
        .content(
            JSON_CONTENT_TYPE,
            ContentBuilder::new().schema(Some(schema)).build(),
        )
        .build()
}

/// Returns the documented `X-Request-Id` response header.
#[must_use]
pub fn request_id_header() -> Header {
    HeaderBuilder::new()
        .schema(Some(ObjectBuilder::new().schema_type(Type::String)))
        .description(Some(
            "Request identifier. Error bodies repeat it as `requestId`; quote it when reporting a \
             problem.",
        ))
        .build()
}

/// Returns the documented `Retry-After` response header in delta seconds.
#[must_use]
pub fn retry_after_header() -> Header {
    HeaderBuilder::new()
        .schema(Some(
            ObjectBuilder::new()
                .schema_type(Type::Integer)
                .minimum(Some(0)),
        ))
        .description(Some("Seconds to wait before retrying."))
        .build()
}

/// Returns the documented `WWW-Authenticate` response header for a bearer challenge.
#[must_use]
pub fn www_authenticate_header() -> Header {
    HeaderBuilder::new()
        .schema(Some(ObjectBuilder::new().schema_type(Type::String)))
        .description(Some("Bearer authentication challenge."))
        .build()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use utoipa::openapi::OpenApi;
    use utoipa::openapi::path::HttpMethod;

    use super::{
        ErrorResponseRules, OperationCondition, REQUEST_ID_HEADER, RETRY_AFTER_HEADER,
        ResponseSelector, WWW_AUTHENTICATE_HEADER, request_id_header,
    };
    use crate::{IfMatchRequirement, document_if_match};

    const ENVELOPE: &str = "#/components/schemas/ErrorEnvelope";

    fn ok() -> Value {
        json!({"200": {"description": "OK"}})
    }

    fn document(paths: Value) -> OpenApi {
        serde_json::from_value(json!({
            "openapi": "3.1.0",
            "info": {"title": "test", "version": "1"},
            "paths": paths
        }))
        .expect("test document")
    }

    fn to_json(document: &OpenApi) -> Value {
        serde_json::to_value(document).expect("serialize")
    }

    fn has_status(document: &Value, path: &str, method: &str, status: &str) -> bool {
        document["paths"][path][method]["responses"]
            .get(status)
            .is_some()
    }

    fn matched(condition: OperationCondition, paths: Value) -> Vec<(String, String)> {
        let mut document = document(paths);
        ErrorResponseRules::new()
            .status(condition, 418, "Matched")
            .apply(&mut document);
        let json = to_json(&document);
        let mut matched = Vec::new();
        for (path, item) in json["paths"].as_object().expect("paths") {
            for method in item.as_object().expect("item").keys() {
                if has_status(&json, path, method, "418") {
                    matched.push((method.clone(), path.clone()));
                }
            }
        }
        matched.sort();
        matched
    }

    fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
        expected
            .iter()
            .map(|(method, path)| ((*method).to_owned(), (*path).to_owned()))
            .collect()
    }

    #[test]
    fn status_rule_adds_an_envelope_response_with_the_product_description() {
        let mut document = document(json!({"/items": {"get": {"responses": ok()}}}));

        ErrorResponseRules::new()
            .status(OperationCondition::Always, 500, "Internal failure")
            .apply(&mut document);

        let response = &to_json(&document)["paths"]["/items"]["get"]["responses"]["500"];
        assert_eq!(response["description"], "Internal failure");
        assert_eq!(
            response["content"]["application/json"]["schema"]["$ref"],
            ENVELOPE
        );
    }

    #[test]
    fn status_rule_keeps_a_status_the_handler_documents() {
        let mut document = document(json!({"/items": {"post": {"responses": {
            "400": {"description": "Invalid item"}
        }}}}));

        ErrorResponseRules::new()
            .status(OperationCondition::Always, 400, "Generic")
            .apply(&mut document);

        assert_eq!(
            to_json(&document)["paths"]["/items"]["post"]["responses"]["400"],
            json!({"description": "Invalid item"})
        );
    }

    #[test]
    fn secured_follows_operation_then_document_security_and_ignores_anonymous_only() {
        let mut document = document(json!({
            "/inherited": {"get": {"responses": ok()}},
            "/own": {"get": {"responses": ok(), "security": [{"apiKey": []}]}},
            "/public": {"get": {"responses": ok(), "security": []}},
            "/anonymous": {"get": {"responses": ok(), "security": [{}]}},
            "/optional": {"get": {"responses": ok(), "security": [{}, {"bearer": []}]}}
        }));
        document.security = serde_json::from_value(json!([{"bearer": []}])).expect("security");

        ErrorResponseRules::new()
            .status(OperationCondition::Secured, 401, "Unauthenticated")
            .apply(&mut document);

        let json = to_json(&document);
        for (path, secured) in [
            ("/inherited", true),
            ("/own", true),
            ("/public", false),
            ("/anonymous", false),
            ("/optional", true),
        ] {
            assert_eq!(has_status(&json, path, "get", "401"), secured, "{path}");
        }
    }

    #[test]
    fn secured_is_false_without_any_security() {
        let paths = json!({"/items": {"get": {"responses": ok()}}});

        assert!(matched(OperationCondition::Secured, paths).is_empty());
    }

    #[test]
    fn request_body_path_parameter_and_unsafe_method_conditions() {
        let paths = json!({
            "/items": {
                "get": {"responses": ok()},
                "post": {"responses": ok(), "requestBody": {"content": {}}}
            },
            "/items/{id}": {
                "get": {"responses": ok()},
                "put": {"responses": ok()},
                "patch": {"responses": ok()},
                "delete": {"responses": ok()},
                "head": {"responses": ok()},
                "options": {"responses": ok()}
            }
        });

        assert_eq!(
            matched(OperationCondition::HasRequestBody, paths.clone()),
            pairs(&[("post", "/items")])
        );
        assert_eq!(
            matched(OperationCondition::HasPathParameter, paths.clone()).len(),
            6
        );
        assert_eq!(
            matched(OperationCondition::UnsafeMethod, paths),
            pairs(&[
                ("delete", "/items/{id}"),
                ("patch", "/items/{id}"),
                ("post", "/items"),
                ("put", "/items/{id}"),
            ])
        );
    }

    #[test]
    fn idempotency_key_condition_matches_the_header_without_regard_to_case() {
        let paths = json!({
            "/a": {"post": {"responses": ok(), "parameters": [
                {"name": "idempotency-key", "in": "header", "required": false}
            ]}},
            "/b": {"post": {"responses": ok(), "parameters": [
                {"name": "Idempotency-Key", "in": "query", "required": false}
            ]}},
            "/c": {"post": {"responses": ok()}}
        });

        assert_eq!(
            matched(OperationCondition::HasIdempotencyKey, paths),
            pairs(&[("post", "/a")])
        );
    }

    #[test]
    fn standard_headers_document_request_id_retry_after_and_challenge() {
        let mut document = document(json!({"/items": {"get": {
            "responses": ok(),
            "security": [{"bearer": []}]
        }}}));

        ErrorResponseRules::new()
            .status(OperationCondition::Secured, 401, "Unauthenticated")
            .status(OperationCondition::Always, 429, "Too many requests")
            .standard_headers()
            .apply(&mut document);

        let responses = &to_json(&document)["paths"]["/items"]["get"]["responses"];
        for status in ["200", "401", "429"] {
            assert_eq!(
                responses[status]["headers"][REQUEST_ID_HEADER]["schema"],
                json!({"type": "string"}),
                "{status}"
            );
        }
        assert_eq!(
            responses["429"]["headers"][RETRY_AFTER_HEADER]["schema"],
            json!({"type": "integer", "minimum": 0})
        );
        assert!(responses["401"]["headers"][WWW_AUTHENTICATE_HEADER].is_object());
        assert!(
            responses["200"]["headers"]
                .get(RETRY_AFTER_HEADER)
                .is_none()
        );
        assert!(
            responses["429"]["headers"]
                .get(WWW_AUTHENTICATE_HEADER)
                .is_none()
        );
    }

    #[test]
    fn header_rule_keeps_an_existing_header_of_any_case_and_skips_references() {
        let mut document = document(json!({"/items": {"get": {"responses": {
            "200": {"description": "OK", "headers": {
                "x-request-id": {"schema": {"type": "string"}, "description": "Product"}
            }},
            "500": {"$ref": "#/components/responses/Internal"}
        }}}}));

        ErrorResponseRules::new()
            .header(
                OperationCondition::Always,
                ResponseSelector::Every,
                REQUEST_ID_HEADER,
                request_id_header(),
            )
            .apply(&mut document);

        let responses = &to_json(&document)["paths"]["/items"]["get"]["responses"];
        assert_eq!(
            responses["200"]["headers"],
            json!({"x-request-id": {"schema": {"type": "string"}, "description": "Product"}})
        );
        assert_eq!(
            responses["500"],
            json!({"$ref": "#/components/responses/Internal"})
        );
    }

    #[test]
    fn applying_twice_changes_nothing() {
        let rules = ErrorResponseRules::new()
            .status(OperationCondition::Always, 400, "Invalid")
            .status(OperationCondition::HasPathParameter, 404, "Missing")
            .standard_headers();
        let mut document = document(json!({"/items/{id}": {"get": {"responses": ok()}}}));

        rules.apply(&mut document);
        let once = to_json(&document);
        rules.apply(&mut document);

        assert_eq!(to_json(&document), once);
    }

    #[test]
    fn apply_where_limits_rules_to_selected_operations() {
        let mut document = document(json!({
            "/items/{id}": {"get": {"responses": ok()}, "patch": {"responses": ok()}},
            "/settings": {"patch": {"responses": ok()}}
        }));

        ErrorResponseRules::new()
            .status(OperationCondition::Always, 412, "Stale revision")
            .apply_where(&mut document, |path, method| {
                *method == HttpMethod::Patch && path.starts_with("/items/")
            });

        let json = to_json(&document);
        assert!(has_status(&json, "/items/{id}", "patch", "412"));
        assert!(!has_status(&json, "/items/{id}", "get", "412"));
        assert!(!has_status(&json, "/settings", "patch", "412"));
    }

    #[test]
    fn precondition_responses_keep_their_descriptions_and_gain_rule_headers() {
        let mut document = document(json!({"/items/{id}": {"put": {"responses": ok()}}}));
        let operation = document
            .paths
            .paths
            .get_mut("/items/{id}")
            .and_then(|item| item.put.as_mut())
            .expect("put");
        document_if_match(operation, IfMatchRequirement::Required);
        let before = to_json(&document)["paths"]["/items/{id}"]["put"]["responses"].clone();

        ErrorResponseRules::new()
            .status(OperationCondition::Always, 400, "Generic")
            .status(OperationCondition::Always, 412, "Generic")
            .standard_headers()
            .apply(&mut document);

        let after = &to_json(&document)["paths"]["/items/{id}"]["put"]["responses"];
        for status in ["400", "412", "428"] {
            assert_eq!(
                after[status]["description"], before[status]["description"],
                "{status}"
            );
            assert_eq!(
                after[status]["content"], before[status]["content"],
                "{status}"
            );
            assert!(after[status]["headers"][REQUEST_ID_HEADER].is_object());
        }
    }
}
