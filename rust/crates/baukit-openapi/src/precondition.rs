use utoipa::openapi::header::{Header, HeaderBuilder};
use utoipa::openapi::path::{Operation, Parameter, ParameterBuilder, ParameterIn};
use utoipa::openapi::{ObjectBuilder, RefOr, Required, Type};

use crate::error_responses::insert_error_response;

/// The request header that carries a revision precondition.
pub const IF_MATCH_HEADER: &str = "If-Match";

/// The response header that carries a strong revision validator.
pub const ETAG_HEADER: &str = "ETag";

/// Error code for a required `If-Match` header that is missing (HTTP 428).
pub const PRECONDITION_REQUIRED_CODE: &str = "precondition_required";

/// Error code for an `If-Match` revision that is no longer current (HTTP 412).
pub const PRECONDITION_FAILED_CODE: &str = "precondition_failed";

/// Error code for an `If-Match` value that is not one strong ETag for the resource (HTTP 400).
pub const INVALID_IF_MATCH_CODE: &str = "invalid_if_match";

const STATUS_BAD_REQUEST: &str = "400";
const STATUS_PRECONDITION_FAILED: &str = "412";
const STATUS_PRECONDITION_REQUIRED: &str = "428";
const SUCCESS_STATUS_PREFIX: char = '2';

/// Whether a route rejects a write that omits `If-Match`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IfMatchRequirement {
    /// A missing header is rejected with 428.
    Required,
    /// A missing header allows an unconditional write.
    Optional,
}

impl IfMatchRequirement {
    const fn is_required(self) -> bool {
        matches!(self, Self::Required)
    }
}

/// Returns the documented `If-Match` header parameter for a revision precondition.
#[must_use]
pub fn if_match_parameter(requirement: IfMatchRequirement) -> Parameter {
    let missing = match requirement {
        IfMatchRequirement::Required => {
            format!("A missing header returns 428 `{PRECONDITION_REQUIRED_CODE}`.")
        }
        IfMatchRequirement::Optional => "Omit it to write unconditionally.".to_owned(),
    };
    let required = if requirement.is_required() {
        Required::True
    } else {
        Required::False
    };
    ParameterBuilder::new()
        .name(IF_MATCH_HEADER)
        .parameter_in(ParameterIn::Header)
        .required(required)
        .description(Some(format!(
            "The strong ETag from the latest read of this resource. {missing} A stale ETag \
             returns 412 `{PRECONDITION_FAILED_CODE}`. `*`, weak ETags, lists, and ETags of \
             another resource return 400 `{INVALID_IF_MATCH_CODE}`."
        )))
        .schema(Some(ObjectBuilder::new().schema_type(Type::String)))
        .build()
}

/// Returns the documented `ETag` response header for a strong revision validator.
#[must_use]
pub fn etag_header() -> Header {
    HeaderBuilder::new()
        .schema(ObjectBuilder::new().schema_type(Type::String))
        .description(Some(
            "Strong revision validator. Send it unchanged in If-Match to update or delete this \
             resource.",
        ))
        .build()
}

/// Documents a revision precondition on one operation.
///
/// Replaces any existing `If-Match` header parameter, matched without regard to case, and adds
/// 400, 412, and, for required routes, 428 responses with the shared error envelope. Responses
/// the operation already documents are kept.
pub fn document_if_match(operation: &mut Operation, requirement: IfMatchRequirement) {
    let parameters = operation.parameters.get_or_insert_with(Vec::new);
    parameters.retain(|parameter| !is_if_match(parameter));
    parameters.push(if_match_parameter(requirement));

    insert_error_response(
        operation,
        STATUS_BAD_REQUEST,
        &format!("`{INVALID_IF_MATCH_CODE}`: If-Match is not one strong ETag for this resource"),
    );
    insert_error_response(
        operation,
        STATUS_PRECONDITION_FAILED,
        &format!("`{PRECONDITION_FAILED_CODE}`: the resource changed since it was read"),
    );
    if requirement.is_required() {
        insert_error_response(
            operation,
            STATUS_PRECONDITION_REQUIRED,
            &format!("`{PRECONDITION_REQUIRED_CODE}`: If-Match is required"),
        );
    }
}

/// Documents the `ETag` header on every inline 2xx response of one operation.
///
/// Responses given as a `$ref` are left unchanged, because they are shared components.
pub fn document_etag(operation: &mut Operation) {
    for (status, response) in &mut operation.responses.responses {
        if !status.starts_with(SUCCESS_STATUS_PREFIX) {
            continue;
        }
        if let RefOr::T(response) = response {
            response
                .headers
                .insert(ETAG_HEADER.to_owned(), etag_header());
        }
    }
}

fn is_if_match(parameter: &Parameter) -> bool {
    parameter.parameter_in == ParameterIn::Header
        && parameter.name.eq_ignore_ascii_case(IF_MATCH_HEADER)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use utoipa::OpenApi as _;

    use super::{
        ETAG_HEADER, IF_MATCH_HEADER, IfMatchRequirement, document_etag, document_if_match,
    };
    use crate::{ErrorEnvelope, serialize_schema};

    #[derive(serde::Serialize, utoipa::ToSchema)]
    struct Item {
        name: String,
    }

    #[utoipa::path(
        put,
        path = "/items/{id}",
        params(
            ("id" = String, Path, description = "Item id"),
            ("if-match" = Option<String>, Header, description = "Hand-written")
        ),
        responses(
            (status = 200, description = "Updated", body = Item),
            (status = 400, description = "Invalid item", body = ErrorEnvelope),
            (status = 404, description = "Missing", body = ErrorEnvelope)
        )
    )]
    #[allow(dead_code)]
    fn update_item() {}

    #[utoipa::path(
        patch,
        path = "/settings",
        responses((status = 200, description = "Updated", body = Item))
    )]
    #[allow(dead_code)]
    fn patch_settings() {}

    #[utoipa::path(
        get,
        path = "/items/{id}",
        params(("id" = String, Path, description = "Item id")),
        responses(
            (status = 200, description = "Found", body = Item),
            (status = 404, description = "Missing", body = ErrorEnvelope)
        )
    )]
    #[allow(dead_code)]
    fn get_item() {}

    #[derive(utoipa::OpenApi)]
    #[openapi(
        paths(update_item, patch_settings, get_item),
        components(schemas(ErrorEnvelope, Item))
    )]
    struct ApiDoc;

    fn documented() -> Value {
        let mut document = ApiDoc::openapi();
        let paths = &mut document.paths.paths;
        let item = paths.get_mut("/items/{id}").expect("item path");
        document_if_match(
            item.put.as_mut().expect("put"),
            IfMatchRequirement::Required,
        );
        document_etag(item.put.as_mut().expect("put"));
        document_etag(item.get.as_mut().expect("get"));
        let settings = paths.get_mut("/settings").expect("settings path");
        document_if_match(
            settings.patch.as_mut().expect("patch"),
            IfMatchRequirement::Optional,
        );
        let json = serialize_schema(&document).expect("serializable");
        serde_json::from_str(&json).expect("valid JSON")
    }

    fn if_match_parameters(operation: &Value) -> Vec<&Value> {
        operation["parameters"]
            .as_array()
            .expect("parameters")
            .iter()
            .filter(|parameter| {
                parameter["in"] == "header"
                    && parameter["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(IF_MATCH_HEADER))
            })
            .collect()
    }

    #[test]
    fn required_precondition_replaces_hand_written_parameter_and_adds_errors() {
        let document = documented();
        let put = &document["paths"]["/items/{id}"]["put"];

        let parameters = if_match_parameters(put);
        assert_eq!(parameters.len(), 1);
        assert_eq!(parameters[0]["name"], IF_MATCH_HEADER);
        assert_eq!(parameters[0]["required"], true);
        assert_eq!(parameters[0]["schema"], json!({"type": "string"}));
        let description = parameters[0]["description"].as_str().expect("description");
        for code in [
            "precondition_required",
            "precondition_failed",
            "invalid_if_match",
        ] {
            assert!(description.contains(code), "{code}");
        }

        let responses = &put["responses"];
        assert_eq!(responses["400"]["description"], "Invalid item");
        for status in ["412", "428"] {
            assert_eq!(
                responses[status]["content"]["application/json"]["schema"]["$ref"],
                "#/components/schemas/ErrorEnvelope",
                "{status}"
            );
        }
        assert_eq!(responses["404"]["description"], "Missing");
    }

    #[test]
    fn optional_precondition_documents_no_428() {
        let document = documented();
        let patch = &document["paths"]["/settings"]["patch"];

        let parameters = if_match_parameters(patch);
        assert_eq!(parameters.len(), 1);
        assert_eq!(parameters[0]["required"], false);
        let responses = patch["responses"].as_object().expect("responses");
        assert!(responses.contains_key("400"));
        assert!(responses.contains_key("412"));
        assert!(!responses.contains_key("428"));
    }

    #[test]
    fn etag_is_documented_on_success_responses_only() {
        let document = documented();
        for method in ["put", "get"] {
            let responses = &document["paths"]["/items/{id}"][method]["responses"];
            assert_eq!(
                responses["200"]["headers"][ETAG_HEADER]["schema"],
                json!({"type": "string"}),
                "{method}"
            );
            assert!(responses["404"].get("headers").is_none(), "{method}");
        }
        let settings = &document["paths"]["/settings"]["patch"]["responses"]["200"];
        assert!(settings.get("headers").is_none());
    }

    #[test]
    fn documenting_twice_is_stable() {
        let mut document = ApiDoc::openapi();
        let item = document
            .paths
            .paths
            .get_mut("/items/{id}")
            .expect("item path");
        let put = item.put.as_mut().expect("put");
        document_if_match(put, IfMatchRequirement::Optional);
        document_if_match(put, IfMatchRequirement::Required);
        document_etag(put);
        document_etag(put);
        let operation = serde_json::to_value(put.clone()).expect("serializable");
        assert_eq!(if_match_parameters(&operation).len(), 1);
        assert_eq!(if_match_parameters(&operation)[0]["required"], true);
        assert_eq!(
            operation["responses"]["200"]["headers"]
                .as_object()
                .expect("headers")
                .len(),
            1
        );
    }
}
