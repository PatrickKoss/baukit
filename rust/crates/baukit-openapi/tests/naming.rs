use baukit_openapi::{
    ErrorEnvelope, NameKind, NamingViolation, assert_camel_case_names, check_camel_case_names,
    find_naming_violations, is_camel_case,
};
use serde_json::{Value, json};
use utoipa::OpenApi;

fn pointers(violations: &[NamingViolation]) -> Vec<&str> {
    violations
        .iter()
        .map(|violation| violation.pointer.as_str())
        .collect()
}

fn parse(document: &str) -> Value {
    serde_json::from_str(document).expect("fixture is valid JSON")
}

#[test]
fn camel_case_accepts_lower_camel_names_only() {
    for name in ["id", "requestId", "nextCursor", "sha256Digest", "userID"] {
        assert!(is_camel_case(name), "{name}");
    }
    for name in [
        "",
        "request_id",
        "RequestId",
        "request-id",
        "_id",
        "1st",
        "naïve",
    ] {
        assert!(!is_camel_case(name), "{name}");
    }
}

#[test]
fn reports_properties_and_path_and_query_parameters_with_pointers() {
    let document = json!({
        "paths": {
            "/runs/{run_id}": {
                "parameters": [
                    {"in": "path", "name": "run_id", "schema": {"type": "string"}},
                    {"in": "header", "name": "x_trace", "schema": {"type": "string"}},
                    {"in": "cookie", "name": "session_id", "schema": {"type": "string"}}
                ],
                "get": {
                    "parameters": [{"in": "query", "name": "page_size", "schema": {"type": "integer"}}],
                    "requestBody": {"content": {"multipart/form-data": {"schema": {
                        "type": "object",
                        "properties": {"upload_file": {"type": "string", "format": "binary"}}
                    }}}}
                }
            }
        },
        "components": {
            "parameters": {"Cursor": {"in": "query", "name": "cursor_token"}},
            "schemas": {"Run": {
                "type": "object",
                "properties": {
                    "runId": {"type": "string"},
                    "started_at": {"type": "string"},
                    "nested": {"type": "object", "properties": {"inner_value": {"type": "integer"}}}
                },
                "required": ["runId", "started_at"]
            }}
        }
    });

    let violations = find_naming_violations(&document, &[]);

    assert_eq!(
        pointers(&violations),
        [
            "/components/parameters/Cursor",
            "/components/schemas/Run/properties/nested/properties/inner_value",
            "/components/schemas/Run/properties/started_at",
            "/paths/~1runs~1{run_id}/get/parameters/0",
            "/paths/~1runs~1{run_id}/get/requestBody/content/multipart~1form-data/schema/properties/upload_file",
            "/paths/~1runs~1{run_id}/parameters/0",
        ]
    );
    let kinds: Vec<_> = violations.iter().map(|violation| violation.kind).collect();
    assert_eq!(
        kinds,
        [
            NameKind::Parameter,
            NameKind::Property,
            NameKind::Property,
            NameKind::Parameter,
            NameKind::Property,
            NameKind::Parameter,
        ]
    );
    assert_eq!(violations[0].name, "cursor_token");
}

#[test]
fn skips_values_maps_examples_and_extensions() {
    let document = json!({
        "components": {
            "examples": {"Sample": {"value": {"snake_key": 1}}},
            "schemas": {
                "Status": {"type": "string", "enum": ["needs_reconnect", "rate_limited"]},
                "Fixed": {"const": {"fixed_key": true}},
                "Counts": {
                    "type": "object",
                    "additionalProperties": {"type": "integer"},
                    "propertyNames": {"type": "string"}
                },
                "Tagged": {
                    "type": "object",
                    "properties": {
                        "kind": {"type": "string"},
                        "properties": {"type": "object", "additionalProperties": {"type": "string"}},
                        "default": {"type": "string", "default": "snake_value"}
                    },
                    "discriminator": {"propertyName": "kind", "mapping": {"some_kind": "#/x"}},
                    "example": {"snake_key": "value"},
                    "examples": [{"snake_key": "value"}],
                    "x-internal": {"properties": {"snake_key": {}}}
                }
            }
        }
    });

    assert!(find_naming_violations(&document, &[]).is_empty());
}

#[test]
fn map_values_with_their_own_properties_are_still_checked() {
    let document = json!({"components": {"schemas": {"Groups": {
        "type": "object",
        "additionalProperties": {"type": "object", "properties": {"member_count": {"type": "integer"}}}
    }}}});

    assert_eq!(
        pointers(&find_naming_violations(&document, &[])),
        ["/components/schemas/Groups/additionalProperties/properties/member_count"]
    );
}

#[test]
fn exemptions_accept_standard_defined_names_everywhere() {
    let document = json!({"components": {"schemas": {"TokenResponse": {
        "type": "object",
        "properties": {
            "access_token": {"type": "string"},
            "token_type": {"type": "string"},
            "expires_in": {"type": "integer"},
            "refresh_hint": {"type": "string"}
        }
    }}}});

    let violations =
        find_naming_violations(&document, &["access_token", "token_type", "expires_in"]);

    assert_eq!(
        pointers(&violations),
        ["/components/schemas/TokenResponse/properties/refresh_hint"]
    );
}

#[test]
fn template_schemas_pass() {
    for document in [
        include_str!("../../../../templates/backend/backend/openapi.json"),
        include_str!("../../../../templates/backend/__auth__/backend/openapi.json"),
    ] {
        assert_eq!(find_naming_violations(&parse(document), &[]), []);
    }
}

#[test]
fn product_excerpt_reports_each_snake_case_name() {
    let document = parse(include_str!("fixtures/snake-case-excerpt.json"));

    let violations = find_naming_violations(&document, &[]);

    let names: Vec<_> = violations
        .iter()
        .map(|violation| violation.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "hour_histogram",
            "mood_tags",
            "template_runs",
            "request_id",
            "next_cursor",
            "completed_at",
            "entry_id",
            "program_id",
            "started_at",
            "body_markdown",
            "expected_revision",
            "body_markdown",
            "section_id",
            "program_id",
            "run_id",
            "section_id",
        ]
    );
    assert!(violations.iter().any(|violation| {
        violation.pointer == "/paths/~1v1~1program-runs/get/parameters/1"
            && violation.kind == NameKind::Parameter
    }));
}

#[derive(OpenApi)]
#[openapi(components(schemas(ErrorEnvelope)))]
struct ErrorOnlyDoc;

#[derive(serde::Serialize, utoipa::ToSchema)]
struct SnakeDto {
    created_at: String,
}

#[derive(OpenApi)]
#[openapi(components(schemas(SnakeDto)))]
struct SnakeDoc;

#[test]
fn checks_a_generated_document() {
    let error_only = ErrorOnlyDoc::openapi();
    check_camel_case_names(&error_only, &[]).expect("the error envelope is camelCase");
    assert_camel_case_names(&error_only, &[]);

    let error = check_camel_case_names(&SnakeDoc::openapi(), &[])
        .expect_err("a snake_case field must fail");
    assert_eq!(
        pointers(error.naming_violations()),
        ["/components/schemas/SnakeDto/properties/created_at"]
    );
    assert!(error.to_string().contains("created_at"), "{error}");
    assert!(!error.is_drift());
    check_camel_case_names(&SnakeDoc::openapi(), &["created_at"]).expect("exempted");
}

#[test]
#[should_panic(expected = "not camelCase")]
fn assertion_panics_with_the_violations() {
    assert_camel_case_names(&SnakeDoc::openapi(), &[]);
}
