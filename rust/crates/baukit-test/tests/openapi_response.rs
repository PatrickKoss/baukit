use axum::{body::to_bytes, http::header::CONTENT_TYPE, response::IntoResponse};
use baukit_http::{ApiError, ErrorBody, ErrorEnvelope};
use baukit_openapi::{ErrorResponseRules, OperationCondition};
use baukit_test::{
    ObservedResponse, OpenApiContractError, assert_response_matches_openapi,
    check_response_matches_openapi,
};
use serde::Serialize;
use serde_json::{Value, json};
use utoipa::{OpenApi, ToSchema};

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
struct WidgetDto {
    #[schema(format = "uuid")]
    id: String,
    display_name: String,
    archived_at: Option<String>,
}

#[utoipa::path(
    get,
    path = "/widgets/{id}",
    params(("id" = String, Path)),
    responses((status = 200, description = "Widget", body = WidgetDto))
)]
#[allow(dead_code)]
async fn get_widget() {}

#[derive(OpenApi)]
#[openapi(
    paths(get_widget),
    components(schemas(WidgetDto, ErrorEnvelope, ErrorBody))
)]
struct WidgetApi;

fn generated_document() -> Value {
    let mut document = WidgetApi::openapi();
    ErrorResponseRules::new()
        .status(OperationCondition::HasPathParameter, 404, "Not found.")
        .apply(&mut document);
    serde_json::to_value(document).expect("document serializes")
}

fn observed<'a>(
    path: &'a str,
    status: u16,
    content_type: Option<&'a str>,
    body: &'a [u8],
) -> ObservedResponse<'a> {
    ObservedResponse {
        method: "GET",
        path,
        status,
        content_type,
        body,
    }
}

fn fixture_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "paths": {
            "/files/{id}": {
                "get": {
                    "responses": {
                        "200": {
                            "description": "File",
                            "content": {
                                "application/pdf": {"schema": {"type": "string", "format": "binary"}},
                                "image/*": {}
                            }
                        },
                        "204": {"description": "Empty"},
                        "4XX": {"$ref": "#/components/responses/Problem"},
                        "default": {
                            "description": "Other",
                            "content": {"application/problem+json": {"schema": {"type": "object", "required": ["title"]}}}
                        }
                    }
                }
            }
        },
        "components": {
            "responses": {
                "Problem": {"$ref": "#/components/responses/Envelope"},
                "Envelope": {
                    "description": "Error",
                    "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Error"}}}
                },
                "Loop": {"$ref": "#/components/responses/Loop"}
            },
            "schemas": {
                "Error": {
                    "type": "object",
                    "required": ["code"],
                    "properties": {"code": {"type": "string"}}
                }
            }
        }
    })
}

#[tokio::test]
async fn real_error_response_matches_the_generated_document() {
    let response = ApiError::not_found("Widget not found").into_response();
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body reads");

    assert_response_matches_openapi(
        &generated_document(),
        &observed("/widgets/{id}", status, content_type.as_deref(), &body),
    );
}

#[test]
fn success_body_matches_nullable_and_format_constraints() {
    let body = json!({
        "id": "0190a6d8-8f43-7c1e-9b35-3f9d7c1b2a10",
        "displayName": "Gear",
        "archivedAt": null
    })
    .to_string();

    assert_response_matches_openapi(
        &generated_document(),
        &observed(
            "/widgets/{id}",
            200,
            Some("application/json; charset=utf-8"),
            body.as_bytes(),
        ),
    );
}

#[test]
fn schema_violations_name_every_failing_value() {
    let body = json!({"id": "not-a-uuid", "archivedAt": 7}).to_string();

    let error = check_response_matches_openapi(
        &generated_document(),
        &observed(
            "/widgets/{id}",
            200,
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
    assert_eq!(location, "GET /widgets/{id} 200 application/json");
    assert_eq!(violations.len(), 3, "{violations:?}");
    assert!(
        violations
            .iter()
            .any(|v| v.starts_with("/: ") && v.contains("displayName"))
    );
    assert!(violations.iter().any(|v| v.starts_with("/id: ")));
    assert!(violations.iter().any(|v| v.starts_with("/archivedAt: ")));
}

#[test]
fn undocumented_operations_and_statuses_are_rejected() {
    let document = generated_document();

    let error = check_response_matches_openapi(&document, &observed("/gadgets", 200, None, b""))
        .expect_err("the path is undocumented");
    assert_eq!(error.to_string(), "GET /gadgets is not documented");

    let error =
        check_response_matches_openapi(&document, &observed("/widgets/{id}", 500, None, b""))
            .expect_err("500 is undocumented");
    assert_eq!(
        error.to_string(),
        "GET /widgets/{id} does not document status 500"
    );
}

#[test]
fn status_class_resolves_chained_response_references() {
    let body = br#"{"code":"forbidden"}"#;

    assert_response_matches_openapi(
        &fixture_document(),
        &observed("/files/{id}", 403, Some("application/json"), body),
    );
    let error = check_response_matches_openapi(
        &fixture_document(),
        &observed("/files/{id}", 403, Some("application/json"), b"{}"),
    )
    .expect_err("code is required");
    assert!(matches!(
        error,
        OpenApiContractError::SchemaViolation { .. }
    ));
}

#[test]
fn default_response_covers_other_statuses_and_json_suffixes() {
    let error = check_response_matches_openapi(
        &fixture_document(),
        &observed(
            "/files/{id}",
            503,
            Some("application/problem+json"),
            b"{\"detail\":\"down\"}",
        ),
    )
    .expect_err("title is required");

    assert!(error.to_string().starts_with(
        "GET /files/{id} 503 application/problem+json: the body violates the documented schema"
    ));
}

#[test]
fn non_json_media_types_match_exactly_or_by_wildcard_without_validation() {
    let document = fixture_document();

    assert_response_matches_openapi(
        &document,
        &observed("/files/{id}", 200, Some("application/pdf"), b"%PDF-1.7"),
    );
    assert_response_matches_openapi(
        &document,
        &observed("/files/{id}", 200, Some("image/png"), b"\x89PNG"),
    );
    let error = check_response_matches_openapi(
        &document,
        &observed("/files/{id}", 200, Some("text/plain"), b"hello"),
    )
    .expect_err("text/plain is undocumented");
    assert_eq!(
        error.to_string(),
        "GET /files/{id} 200: media type text/plain is not documented"
    );
}

#[test]
fn body_presence_must_match_the_documented_content() {
    let document = fixture_document();

    assert_response_matches_openapi(&document, &observed("/files/{id}", 204, None, b""));
    let error = check_response_matches_openapi(
        &document,
        &observed("/files/{id}", 204, Some("application/json"), b"{}"),
    )
    .expect_err("204 documents no body");
    assert!(matches!(
        error,
        OpenApiContractError::UndocumentedBody { .. }
    ));

    let error = check_response_matches_openapi(&document, &observed("/files/{id}", 200, None, b""))
        .expect_err("200 documents a body");
    assert!(matches!(error, OpenApiContractError::MissingBody { .. }));

    let error =
        check_response_matches_openapi(&document, &observed("/files/{id}", 200, None, b"%PDF"))
            .expect_err("a body needs a content type");
    assert!(matches!(
        error,
        OpenApiContractError::MissingContentType { .. }
    ));
}

#[test]
fn invalid_json_and_broken_references_are_reported() {
    let error = check_response_matches_openapi(
        &fixture_document(),
        &observed("/files/{id}", 404, Some("application/json"), b"{"),
    )
    .expect_err("the body is not JSON");
    assert!(matches!(error, OpenApiContractError::InvalidJson { .. }));

    let mut document = fixture_document();
    document["paths"]["/files/{id}"]["get"]["responses"]["4XX"] =
        json!({"$ref": "#/components/responses/Loop"});
    let error = check_response_matches_openapi(
        &document,
        &observed("/files/{id}", 404, Some("application/json"), b"{}"),
    )
    .expect_err("the reference loops");
    assert!(matches!(
        error,
        OpenApiContractError::UnresolvedReference { .. }
    ));
}

#[test]
#[should_panic(expected = "GET /widgets/{id} 200 application/json: the body violates")]
fn assert_panics_with_the_check_message() {
    assert_response_matches_openapi(
        &generated_document(),
        &observed("/widgets/{id}", 200, Some("application/json"), b"[]"),
    );
}
