pub mod check;
mod components;
mod dockerfile;
mod doctor;
mod doctor_auth;
mod doctor_backend;
mod doctor_dependencies;
mod doctor_dev_ports;
mod doctor_files;
mod doctor_frontend;
mod doctor_host;
mod doctor_layout;
mod doctor_mcp;
mod doctor_platform;
mod doctor_ports;
mod doctor_screens;
#[cfg(test)]
mod doctor_tests;
mod gate_settings;
mod generate;
mod identity;
mod manifest;
mod openapi_client;
mod ports;
mod ts_aliases;

pub use ports::{PortDeclaration, PortMapping};

pub use components::{Component, Components};
pub use dockerfile::{
    BackendImage, BackendManifest, BuildInput, Download, ImageVariant, PreBuildStep, RuntimeBinary,
    RuntimeFile, RuntimePackages, WritableDirectory, generate_dockerfile, render_dockerfile,
};
pub use doctor::doctor;
pub use doctor_layout::{DoctorPaths, DoctorWaiver};
pub use generate::{NewOptions, generate_new};
pub use manifest::{
    AnalyticsAdapter, AppManifest, AuthProvider, BaukitDependency, Capabilities, Dependencies,
    MANIFEST_SCHEMA_VERSION, Manifest, OpenApiCompatibility, OpenApiPaths, PlatformManifest,
    QualityManifest, QualityProfile, read_manifest,
};
pub use openapi_client::generate_openapi_client;

use include_dir::{Dir, include_dir};

static BACKEND_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/backend");
static COMMON_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/common");
static MOBILE_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/mobile");
static MCP_REMOTE_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/mcp-remote");
static WEB_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/web");
static PLATFORM_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/platform");
const STRICT_GATE_TEMPLATE: &str = "__strict__/scripts/quality-gate.sh";
static WORKER_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../templates/worker");
const REMOTE_MCP_CLIENT_SCOPES: &str = include_str!("../resources/remote-mcp-client-scopes.json");

pub const TEMPLATE_VERSION: &str = include_str!("../../templates/VERSION").trim_ascii();

const OPENAPI_TYPESCRIPT_PACKAGE: &str = "openapi-typescript@7.13.0";
const MAX_APPLICATION_NAME_LEN: usize = 41;
const PLATFORM_MIGRATION_PATH: &str = "backend/migrations/0009_platform.sql";
const LAST_TEMPLATE_MIGRATION_VERSION: u64 = 8;

const EXPECTED_BACKEND_FILES: &[&str] = &[
    "README.md",
    "Makefile",
    ".dockerignore",
    ".cargo/config.toml",
    "compose.yaml",
    "deploy/values.yaml",
    ".agents/skills/baukit-backend/SKILL.md",
    ".claude/skills/baukit-backend/SKILL.md",
    "scripts/openapi.sh",
    "scripts/openapi-client.sh",
];

const EXPECTED_COMMON_FILES: &[&str] = &[
    "CLAUDE.md",
    "AGENTS.md",
    "CHANGELOG.md",
    ".github/workflows/ci.yml",
    "limits.json",
    "scripts/lockfiles.sh",
    "scripts/preflight.sh",
    "scripts/osv-resolved-lock.py",
    "scripts/osv-resolved-lock.test.py",
];

const EXPECTED_STRICT_FILES: &[&str] = &[
    "scripts/quality-gate.sh",
    "scripts/gate.py",
    "scripts/gate.test.py",
    "scripts/gate_locks.py",
    "scripts/gate_locks.test.py",
];

const ENV_RECONCILIATION_FILES: &[&str] = &[
    "scripts/setup.sh",
    "scripts/reconcile-env.py",
    "scripts/reconcile-env.test.py",
];

const MARKDOWN_LINK_CHECK_FILES: &[&str] = &[
    "scripts/check-markdown-links.py",
    "scripts/check-markdown-links.test.py",
];

const EXPECTED_STRICT_BACKEND_FILES: &[&str] = &[
    "scripts/check-migrations-immutable.sh",
    "scripts/check-migrations-immutable.test.sh",
];

const EXPECTED_MOBILE_FILES: &[&str] = &[
    "mobile/package.json",
    "mobile/app.config.ts",
    "mobile/tsconfig.json",
    "mobile/metro.config.js",
    "mobile/eslint.config.js",
    "mobile/jest.config.cjs",
    "mobile/app/_layout.tsx",
    "mobile/app/(tabs)/_layout.tsx",
    "mobile/app/(tabs)/index.tsx",
    "mobile/scripts/generate-tokens.mjs",
    "mobile/src/action-button.tsx",
    "mobile/src/app-shell.tsx",
    "mobile/src/api.ts",
    "mobile/src/back-or-replace.ts",
    "mobile/src/back-or-replace.test.ts",
    "mobile/src/localization/catalogs.test.ts",
    "mobile/src/localization/de.ts",
    "mobile/src/localization/en.ts",
    "mobile/src/localization/i18n.ts",
    "mobile/src/limits.ts",
    "mobile/src/limits.test.ts",
    "mobile/src/route-heading-focus.ts",
    "mobile/src/route-heading-focus.test.ts",
    "mobile/src/theme.ts",
    "mobile/src/tokens.ts",
    "mobile/src/record-store.ts",
];

const EXPECTED_WEB_FILES: &[&str] = &[
    "scripts/with-loopback-network.sh",
    "scripts/with-loopback-network.test.py",
    "web/package.json",
    "web/index.html",
    "web/vite.config.ts",
    "web/tsconfig.json",
    "web/eslint.config.js",
    "web/vitest.config.ts",
    "web/src/App.tsx",
    "web/src/api.ts",
    "web/src/limits.ts",
    "web/src/limits.test.ts",
    "web/src/tokens.css",
    "web/e2e/playwright.config.ts",
    "web/e2e/qa.config.ts",
    "web/e2e/tsconfig.json",
    "web/e2e/tests/qa.ts",
    "web/e2e/tests/geometry.ts",
    "web/e2e/tests/console-warnings.ts",
    "web/e2e/tests/qa-axe.spec.ts",
    "web/e2e/tests/qa-keyboard.spec.ts",
    "web/e2e/tests/qa-overlay-dismiss.spec.ts",
    "web/e2e/tests/qa-submit-guards.spec.ts",
    "web/e2e/tests/qa-route-state.spec.ts",
    "web/e2e/tests/qa-auth-expiry.spec.ts",
    "web/e2e/tests/qa-auth-isolation.spec.ts",
    "web/e2e/tests/qa-scroll.spec.ts",
    "web/e2e/tests/qa-geometry.spec.ts",
    "web/e2e/tests/qa-console.spec.ts",
];

const EXPECTED_AUTH_WEB_FILES: &[&str] = &[
    "web/src/auth.ts",
    "web/src/auth.test.ts",
    "web/src/local-data.ts",
    "web/src/persistence-lifecycle.ts",
];

const EXPECTED_TYPESCRIPT_DEPENDENCIES: &[&str] = &[
    "@baukit/a11y-core",
    "@baukit/api-runtime",
    "@baukit/data-contracts",
    "@baukit/navigation",
    "@baukit/ui-tokens",
];

const EXPECTED_MOBILE_TYPESCRIPT_DEPENDENCIES: &[&str] = &[
    "@baukit/data-contracts-expo-sqlite",
    "@baukit/localization-core",
    "@baukit/preferences-core",
];

/// Generated with every mobile app; doctor does not require them in existing products.
const MOBILE_PURCHASES_DEPENDENCIES: &[&str] =
    &["@baukit/purchases-core", "@baukit/purchases-revenuecat"];

const EXPECTED_MOBILE_AUTH_DEPENDENCIES: &[&str] = &["@baukit/auth-native"];
const EXPECTED_WEB_AUTH_DEPENDENCIES: &[&str] = &["@baukit/auth-web"];
const EXPECTED_WEB_AUTH_DEV_DEPENDENCIES: &[&str] = &["@baukit/auth-node"];
