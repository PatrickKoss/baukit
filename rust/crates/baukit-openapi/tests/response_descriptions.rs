use baukit_openapi::{OpenApiMetadata, fill_missing_response_descriptions, serialize_schema};
use serde_json::{Value, json};
use utoipa::openapi::OpenApi;

fn document() -> OpenApi {
    serde_json::from_value(json!({
        "openapi": "3.1.0",
        "info": {"title": "test", "version": "1"},
        "paths": {
            "/items": {
                "get": {"responses": {
                    "200": {"description": "", "content": {"application/json": {"schema": {"type": "string"}}}},
                    "400": {"description": "Product validation error"},
                    "500": {"$ref": "#/components/responses/Failure"},
                    "default": {}, "4XX": {}, "599": {}
                }, "callbacks": {
                    "updated": {"{$request.body#/callback}": {"post": {"responses": {"202": {}}}}},
                    "shared": {"$ref": "#/components/callbacks/Shared"}
                }},
                "put": {"responses": {"201": {}}},
                "post": {"responses": {"202": {}}},
                "delete": {"responses": {"204": {}}},
                "options": {"responses": {"204": {}}},
                "head": {"responses": {"200": {}}},
                "patch": {"responses": {"200": {}}},
                "trace": {"responses": {"200": {}}}
            }
        },
        "webhooks": {"changed": {"post": {"responses": {"204": {}}}}},
        "components": {
            "responses": {"Failure": {}, "Alias": {"$ref": "#/components/responses/Failure"}},
            "callbacks": {"Shared": {"{$request.body#/url}": {"post": {"responses": {"200": {}}}}}},
            "pathItems": {"SharedPath": {"get": {"responses": {"404": {}}}}}
        }
    })).expect("OpenAPI fixture")
}

fn assert_response_descriptions(value: &Value) -> usize {
    match value {
        Value::Object(object) => object
            .iter()
            .map(|(key, child)| {
                let responses = if key == "responses" {
                    let responses = child.as_object().expect("response map");
                    for response in responses.values() {
                        if response.get("$ref").is_none() {
                            assert!(
                                response
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .is_some_and(|description| !description.is_empty()),
                                "{response}"
                            );
                        }
                    }
                    responses.len()
                } else {
                    0
                };
                responses + assert_response_descriptions(child)
            })
            .sum(),
        Value::Array(array) => array.iter().map(assert_response_descriptions).sum(),
        _ => 0,
    }
}

#[test]
fn fills_every_response_context_and_preserves_product_content() {
    let mut document = document();
    let before = serde_json::to_value(&document).expect("serialize");
    assert!(
        before["paths"]["/items"]["get"]["responses"]["200"]
            .get("description")
            .is_none()
    );
    fill_missing_response_descriptions(&mut document);
    let after = serde_json::to_value(&document).expect("serialize");
    assert_eq!(after["openapi"], "3.1.0");
    assert_eq!(assert_response_descriptions(&after), 19);
    let responses = &after["paths"]["/items"]["get"]["responses"];
    assert_eq!(responses["200"]["description"], "OK");
    assert_eq!(
        responses["200"]["content"],
        before["paths"]["/items"]["get"]["responses"]["200"]["content"]
    );
    assert_eq!(responses["400"]["description"], "Product validation error");
    assert_eq!(
        responses["500"],
        before["paths"]["/items"]["get"]["responses"]["500"]
    );
    for status in ["default", "4XX", "599"] {
        assert_eq!(responses[status]["description"], "Response");
    }
    assert_eq!(
        after["paths"]["/items"]["delete"]["responses"]["204"]["description"],
        "No Content"
    );
    assert_eq!(
        after["components"]["pathItems"]["SharedPath"]["get"]["responses"]["404"]["description"],
        "Not Found"
    );
    fill_missing_response_descriptions(&mut document);
    assert_eq!(serde_json::to_value(&document).expect("serialize"), after);
}

#[test]
fn metadata_and_serialization_fill_descriptions_by_default() {
    let mut document = document();
    let serialized: Value =
        serde_json::from_str(&serialize_schema(&document).expect("schema")).expect("JSON");
    assert_eq!(assert_response_descriptions(&serialized), 19);
    let untouched = serde_json::to_value(&document).expect("serialize");
    assert!(
        untouched["paths"]["/items"]["get"]["responses"]["200"]
            .get("description")
            .is_none()
    );
    OpenApiMetadata::new("Test", "1", "API").apply_to(&mut document);
    assert_eq!(
        assert_response_descriptions(&serde_json::to_value(&document).expect("serialize")),
        19
    );
}
