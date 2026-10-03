use std::collections::BTreeMap;

use http::StatusCode;
use utoipa::openapi::{
    OpenApi, RefOr,
    path::{Callback, PathItem},
    response::Response,
};

/// Fills empty response descriptions so utoipa emits OpenAPI's required field.
///
/// Uses the HTTP reason phrase for known status codes. Default, range, unknown
/// status codes, and reusable responses without a status use "Response".
/// Covers paths, webhooks, callbacks, and reusable components. References and
/// non-empty product descriptions stay intact. Applying this twice has no effect.
pub fn fill_missing_response_descriptions(openapi: &mut OpenApi) {
    for item in openapi.paths.paths.values_mut() {
        fill_path_item(item);
    }
    if let Some(webhooks) = &mut openapi.webhooks {
        for item in webhooks.paths.values_mut() {
            fill_path_item(item);
        }
    }
    if let Some(components) = &mut openapi.components {
        fill_responses(&mut components.responses);
        fill_callbacks(&mut components.callbacks);
        for item in components.path_items.values_mut() {
            fill_path_item(item);
        }
    }
}

fn fill_path_item(item: &mut PathItem) {
    for operation in [
        &mut item.get,
        &mut item.put,
        &mut item.post,
        &mut item.delete,
        &mut item.options,
        &mut item.head,
        &mut item.patch,
        &mut item.trace,
        &mut item.query,
    ]
    .into_iter()
    .flatten()
    .chain(item.additional_operations.values_mut())
    {
        fill_responses(&mut operation.responses.responses);
        fill_callbacks(&mut operation.callbacks);
    }
}

fn fill_callbacks(callbacks: &mut BTreeMap<String, RefOr<Callback>>) {
    for callback in callbacks.values_mut() {
        if let RefOr::T(callback) = callback {
            for item in callback.values_mut() {
                if let RefOr::T(item) = item {
                    fill_path_item(item);
                }
            }
        }
    }
}

fn fill_responses(responses: &mut BTreeMap<String, RefOr<Response>>) {
    for (status, response) in responses {
        if let RefOr::T(response) = response
            && response.description.is_empty()
        {
            response.description = StatusCode::from_bytes(status.as_bytes())
                .ok()
                .and_then(|status| status.canonical_reason())
                .unwrap_or("Response")
                .to_owned();
        }
    }
}
