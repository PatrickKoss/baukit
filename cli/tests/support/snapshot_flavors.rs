use std::path::Path;

use baukit_cli::{AuthProvider, NewOptions, QualityProfile};

type ConfigureFlavor = fn(&mut NewOptions);

pub fn snapshot_flavors(parent: &Path) -> impl Iterator<Item = (&'static str, NewOptions)> {
    let flavors: [(&str, ConfigureFlavor); 11] = [
        ("backend", |_| {}),
        ("worker", |o| o.worker = true),
        ("mobile", |o| {
            o.backend = false;
            o.mobile = true;
        }),
        ("mobile-pwa", |o| {
            o.backend = false;
            o.mobile = true;
            o.pwa = true;
        }),
        ("web", |o| {
            o.backend = false;
            o.web = true;
        }),
        ("combined", |o| {
            o.mobile = true;
            o.web = true;
        }),
        ("mcp-remote", |o| {
            o.mcp = true;
            o.auth = Some(AuthProvider::Oidc);
        }),
        ("strict", |o| {
            o.mobile = true;
            o.web = true;
            o.quality = QualityProfile::Strict;
        }),
        ("clerk", |o| {
            o.mobile = true;
            o.web = true;
            o.mcp = true;
            o.auth = Some(AuthProvider::Clerk);
        }),
        ("workos", |o| {
            o.mobile = true;
            o.web = true;
            o.mcp = true;
            o.auth = Some(AuthProvider::Workos);
        }),
        ("auth", |o| {
            o.mobile = true;
            o.web = true;
            o.auth = Some(AuthProvider::Oidc);
        }),
    ];
    flavors.into_iter().map(move |(flavor, configure)| {
        let mut options = NewOptions {
            name: "snapshot-app".to_owned(),
            directory: parent.to_path_buf(),
            backend: true,
            worker: false,
            mobile: false,
            web: false,
            pwa: false,
            mcp: false,
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
