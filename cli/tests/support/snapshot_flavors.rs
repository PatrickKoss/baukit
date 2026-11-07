use std::path::Path;

use baukit_cli::{AuthProvider, Component, Components, NewOptions, QualityProfile};

type ConfigureFlavor = fn(&mut NewOptions);

pub fn snapshot_flavors(parent: &Path) -> impl Iterator<Item = (&'static str, NewOptions)> {
    let flavors: [(&str, ConfigureFlavor); 13] = [
        ("backend", |_| {}),
        ("platform", |o| {
            o.components.insert(Component::Platform);
            o.auth = Some(AuthProvider::Oidc);
            o.components.insert(Component::Worker);
            o.components.insert(Component::Web);
        }),
        ("platform-standalone", |o| {
            o.components.insert(Component::Platform);
            o.components.insert(Component::Web);
        }),
        ("worker", |o| o.components.set(Component::Worker, true)),
        ("mobile", |o| {
            o.components.remove(Component::Backend);
            o.components.insert(Component::Mobile);
        }),
        ("mobile-pwa", |o| {
            o.components.remove(Component::Backend);
            o.components.insert(Component::Mobile);
            o.components.insert(Component::Pwa);
        }),
        ("web", |o| {
            o.components.remove(Component::Backend);
            o.components.insert(Component::Web);
        }),
        ("combined", |o| {
            o.components.insert(Component::Mobile);
            o.components.insert(Component::Web);
        }),
        ("mcp-remote", |o| {
            o.components.insert(Component::Mcp);
            o.auth = Some(AuthProvider::Oidc);
        }),
        ("strict", |o| {
            o.components.insert(Component::Mobile);
            o.components.insert(Component::Web);
            o.quality = QualityProfile::Strict;
        }),
        ("clerk", |o| {
            o.components.insert(Component::Mobile);
            o.components.insert(Component::Web);
            o.components.insert(Component::Mcp);
            o.auth = Some(AuthProvider::Clerk);
        }),
        ("workos", |o| {
            o.components.insert(Component::Mobile);
            o.components.insert(Component::Web);
            o.components.insert(Component::Mcp);
            o.auth = Some(AuthProvider::Workos);
        }),
        ("auth", |o| {
            o.components.insert(Component::Mobile);
            o.components.insert(Component::Web);
            o.auth = Some(AuthProvider::Oidc);
        }),
    ];
    flavors.into_iter().map(move |(flavor, configure)| {
        let mut options = NewOptions {
            name: "snapshot-app".to_owned(),
            directory: parent.to_path_buf(),
            components: Components::from([Component::Backend]),
            auth: None,
            force: false,
            into_existing: false,
            resolve_lockfiles: false,
            baukit_path: None,
            port_offset: 0,
            quality: QualityProfile::Standard,
        };
        configure(&mut options);
        (flavor, options)
    })
}
