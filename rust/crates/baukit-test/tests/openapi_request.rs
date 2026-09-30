use baukit_test::{
    ObservedRequest, OpenApiContractError, assert_request_matches_openapi,
    check_request_matches_openapi,
};
use serde_json::{Value, json};

fn document() -> Value {
    json!({
        "openapi": "3.1.0",
        "paths": {
            "/widgets": {
                "post": {
                    "requestBody": {"$ref": "#/components/requestBodies/CreateWidget"},
                    "responses": {"201": {"description": "Created"}}
                },
                "get": {"responses": {"200": {"description": "Widgets"}}}
            },
            "/widgets/{id}": {
                "patch": {
                    "requestBody": {
                        "content": {"application/merge-patch+json": {"schema": {"$ref": "#/components/schemas/WidgetPatch"}}}
                    },
                    "responses": {"204": {"description": "Updated"}}
                }
            },
            "/widgets/{id}/image": {
                "put": {
                    "requestBody": {"required": true, "content": {"image/*": {}}},
                    "responses": {"204": {"description": "Stored"}}
                }
            }
        },
        "components": {
            "requestBodies": {
                "CreateWidget": {
                    "required": true,
                    "content": {"application/json": {"schema": {"$ref": "#/components/schemas/NewWidget"}}}
                },
                "Loop": {"$ref": "#/components/requestBodies/Loop"}
            },
            "schemas": {
                "NewWidget": {
                    "type": "object",
                    "required": ["displayName"],
                    "additionalProperties": false,
                    "properties": {
                        "displayName": {"type": "string", "minLength": 1},
                        "ownerId": {"type": "string", "format": "uuid"}
                    }
                },
                "WidgetPatch": {
                    "type": "object",
                    "properties": {"displayName": {"type": ["string", "null"]}}
                }
            }
        }
    })
}

fn request<'a>(
    method: &'a str,
    path: &'a str,
    content_type: Option<&'a str>,
    body: &'a [u8],
) -> ObservedRequest<'a> {
    ObservedRequest {
        method,
        path,
        content_type,
        body,
    }
}

#[test]
fn a_referenced_request_body_validates_against_its_schema() {
    let body = json!({"displayName": "Gear", "ownerId": "0190a6d8-8f43-7c1e-9b35-3f9d7c1b2a10"})
        .to_string();

    assert_request_matches_openapi(
        &document(),
        &request(
            "post",
            "/widgets",
            Some("application/json; charset=utf-8"),
            body.as_bytes(),
        ),
    );
}

#[test]
fn schema_violations_name_every_failing_value() {
    let body = json!({"ownerId": "not-a-uuid", "colour": "red"}).to_string();

    let error = check_request_matches_openapi(
        &document(),
        &request(
            "POST",
            "/widgets",
            Some("application/json"),
            body.as_bytes(),
        ),
    )
    .expect_err("the body violates the schema");

    let OpenApiContractError::SchemaViolation {
        location,
        violations,
    } = error
    else {
        panic!("expected a schema violation, got {error}");
    };
    assert_eq!(location, "POST /widgets request application/json");
    assert_eq!(violations.len(), 3, "{violations:?}");
    assert!(violations.iter().any(|v| v.starts_with("/ownerId: ")));
}

#[test]
fn a_required_body_must_be_present_and_an_optional_one_may_be_empty() {
    let document = document();

    let error = check_request_matches_openapi(&document, &request("POST", "/widgets", None, b""))
        .expect_err("the body is required");
    assert_eq!(
        error.to_string(),
        "POST /widgets request: the documented body is missing"
    );

    assert_request_matches_openapi(&document, &request("PATCH", "/widgets/{id}", None, b""));
    assert_request_matches_openapi(
        &document,
        &request(
            "PATCH",
            "/widgets/{id}",
            Some("application/merge-patch+json"),
            br#"{"displayName":null}"#,
        ),
    );
}

#[test]
fn an_operation_without_a_request_body_rejects_a_body() {
    let document = document();

    assert_request_matches_openapi(&document, &request("GET", "/widgets", None, b""));
    let error = check_request_matches_openapi(
        &document,
        &request("GET", "/widgets", Some("application/json"), b"{}"),
    )
    .expect_err("GET documents no body");
    assert!(matches!(
        error,
        OpenApiContractError::UndocumentedBody { .. }
    ));
}

#[test]
fn media_types_match_exactly_or_by_wildcard() {
    let document = document();

    assert_request_matches_openapi(
        &document,
        &request("PUT", "/widgets/{id}/image", Some("image/png"), b"\x89PNG"),
    );
    let error = check_request_matches_openapi(
        &document,
        &request("POST", "/widgets", Some("text/plain"), b"Gear"),
    )
    .expect_err("text/plain is undocumented");
    assert_eq!(
        error.to_string(),
        "POST /widgets request: media type text/plain is not documented"
    );
    let error = check_request_matches_openapi(
        &document,
        &request("POST", "/widgets", None, br#"{"displayName":"Gear"}"#),
    )
    .expect_err("a body needs a content type");
    assert!(matches!(
        error,
        OpenApiContractError::MissingContentType { .. }
    ));
}

#[test]
fn undocumented_operations_and_broken_references_are_reported() {
    let mut document = document();

    let error = check_request_matches_openapi(&document, &request("DELETE", "/widgets", None, b""))
        .expect_err("DELETE is undocumented");
    assert_eq!(error.to_string(), "DELETE /widgets is not documented");

    document["paths"]["/widgets"]["post"]["requestBody"] =
        json!({"$ref": "#/components/requestBodies/Loop"});
    let error = check_request_matches_openapi(
        &document,
        &request("POST", "/widgets", Some("application/json"), b"{}"),
    )
    .expect_err("the reference loops");
    assert!(matches!(
        error,
        OpenApiContractError::UnresolvedReference { .. }
    ));
}

#[test]
#[should_panic(expected = "POST /widgets request application/json: the body violates")]
fn assert_panics_with_the_check_message() {
    assert_request_matches_openapi(
        &document(),
        &request("POST", "/widgets", Some("application/json"), b"[]"),
    );
}
