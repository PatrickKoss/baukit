use super::*;
use utoipa::OpenApi;
#[test]
fn delivery_error_codes_preserve_every_protocol_code() {
    for code in crate::SUITE_ERROR_CODES {
        let mapped = SuiteDeliveryFailureCode::from(*code);
        assert_eq!(mapped.as_str(), *code);
        assert_eq!(serde_json::to_value(mapped).expect("code"), *code);
    }
    assert_eq!(
        SuiteDeliveryFailureCode::from("unknown").as_str(),
        "internal_error"
    );
}
#[test]
fn openapi_error_codes_match_protocol() {
    let document = serde_json::to_value(SuiteOpenApi::openapi()).expect("OpenAPI");
    let schemas = &document["components"]["schemas"];
    assert_eq!(
        schemas["SuiteErrorCode"]["enum"],
        serde_json::json!(crate::SUITE_ERROR_CODES)
    );
    assert_eq!(
        schemas["SuiteErrorBody"]["properties"]["code"]["$ref"],
        "#/components/schemas/SuiteErrorCode"
    );
}
#[test]
fn openapi_uses_the_event_envelope_wire_name_and_generic_app_ids() {
    let document = serde_json::to_value(SuiteOpenApi::openapi()).expect("OpenAPI");
    let schemas = &document["components"]["schemas"];
    let properties = &schemas["SuiteEnvelope"]["properties"];
    assert!(properties.get("sourceApp").is_some());
    assert!(properties.get("source").is_none());
    assert!(
        schemas["StartRequest"]["properties"]["peerApp"]
            .get("enum")
            .is_none()
    );
    assert!(
        document["paths"]["/suite/inbound/{linkId}"]["post"]["responses"]["422"]["description"]
            .as_str()
            .expect("payload rejection description")
            .contains("suite_payload_invalid")
    );
    assert_eq!(document["paths"].as_object().expect("paths").len(), 15);
    assert!(
        schemas["PeerResponse"]["properties"]
            .get("mappableMetrics")
            .is_none()
    );
}
