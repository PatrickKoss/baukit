use baukit_mcp::{
    CapabilityError, Principal, Resource, ResourceContents, ResourceFuture, ResourceService,
    ResourceTemplate, ScopedResource, ScopedResourceTemplate,
};
use serde_json::json;
use std::sync::Mutex;

#[derive(Default)]
pub struct Catalog(pub Mutex<Vec<(String, String)>>);

impl ResourceService for Catalog {
    fn list(&self) -> Vec<ScopedResource> {
        vec![
            ScopedResource {
                resource: Resource::new("product://guide", "guide")
                    .with_title("Guide")
                    .with_description("Account guide")
                    .with_mime_type("application/json"),
                required_scopes: vec!["items:read".into()],
            },
            ScopedResource {
                resource: Resource::new("product://private", "private"),
                required_scopes: vec!["items:read".into(), "private:read".into()],
            },
        ]
    }
    fn templates(&self) -> Vec<ScopedResourceTemplate> {
        vec![
            ScopedResourceTemplate {
                template: ResourceTemplate::new("product://items/{id}", "item")
                    .with_title("Item")
                    .with_description("Owned item")
                    .with_mime_type("application/json"),
                required_scopes: vec!["items:read".into()],
            },
            ScopedResourceTemplate {
                template: ResourceTemplate::new("product://private/{id}", "private-item"),
                required_scopes: vec!["private:read".into()],
            },
        ]
    }
    fn read<'a>(&'a self, principal: &'a Principal, uri: &'a str) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.0
                .lock()
                .expect("reads")
                .push((principal.subject().into(), uri.into()));
            if uri.ends_with("/missing") {
                return Err(CapabilityError::InvalidParams {
                    message: "Item not found".into(),
                    data: Some(json!({"code":"not_found"})),
                });
            }
            if uri.ends_with("/unavailable") {
                return Err(CapabilityError::Internal {
                    message: "Items unavailable".into(),
                    data: None,
                });
            }
            Ok(vec![
                ResourceContents::text(
                    json!({"subject":principal.subject(),"uri":uri}).to_string(),
                    uri,
                )
                .with_mime_type("application/json"),
            ])
        })
    }
}
