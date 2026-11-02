use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

use crate::{Capabilities, QualityManifest};

const FIRST_EMULATOR_PORT: u16 = 5556;
const EMULATOR_PORT_SLOTS: u32 = 63;
const LAST_EMULATOR_PORT: u16 = 5682;
const DEFAULT_ANDROID_EMULATORS: u8 = 2;
const MAX_ANDROID_EMULATORS: u8 = 4;

pub const fn default_android_emulators() -> u8 {
    DEFAULT_ANDROID_EMULATORS
}
const FNV_OFFSET: u32 = 0x811c_9dc5;
const FNV_PRIME: u32 = 0x0100_0193;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GateBudgetMinutes {
    #[serde(default = "default_full_budget")]
    pub full: u16,
    #[serde(default = "default_affected_budget")]
    pub affected: u16,
}

impl Default for GateBudgetMinutes {
    fn default() -> Self {
        Self {
            full: default_full_budget(),
            affected: default_affected_budget(),
        }
    }
}

const fn default_full_budget() -> u16 {
    10
}

const fn default_affected_budget() -> u16 {
    5
}

/// Even console port in 5556..=5680 so products on one host get different emulators.
/// Extra emulators use the following even ports.
pub fn default_emulator_port(product: &str) -> u16 {
    let hash = product.bytes().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(FNV_PRIME)
    });
    let slot = u16::try_from(hash % EMULATOR_PORT_SLOTS).unwrap_or_default();
    FIRST_EMULATOR_PORT + 2 * slot
}

pub fn default_affected(capabilities: &Capabilities) -> BTreeMap<String, Vec<String>> {
    let mut affected = BTreeMap::new();
    let mut add = |glob: &str, tokens: &[&str]| {
        affected.insert(
            glob.to_owned(),
            tokens.iter().map(|token| (*token).to_owned()).collect(),
        );
    };
    for document in [
        "README.md",
        "CLAUDE.md",
        "AGENTS.md",
        "CHANGELOG.md",
        "docs/**",
    ] {
        add(document, &[]);
    }
    if capabilities.backend {
        add("backend/crates/**", &["backend"]);
        add("backend/Cargo.toml", &["backend", "images"]);
        add("backend/Dockerfile", &["images"]);
    }
    if capabilities.web {
        add("web/src/**", &["web"]);
        add("web/public/**", &["web"]);
        add("web/index.html", &["web"]);
        add("web/e2e/**", &["web-e2e"]);
    }
    if capabilities.mobile {
        add("mobile/**", &["mobile"]);
    }
    affected
}

pub fn render_quality_extras(name: &str, capabilities: &Capabilities, strict: bool) -> String {
    let mut text = String::new();
    if capabilities.mobile {
        text.push_str(&format!(
            "android_emulator_port = {}\nandroid_emulators = {}\n",
            default_emulator_port(name),
            default_android_emulators()
        ));
    }
    if !strict {
        return text;
    }
    text.push_str("shared = []\n");
    let budget = GateBudgetMinutes::default();
    text.push_str(&format!(
        "\n[quality.gate_budget_minutes]\nfull = {}\naffected = {}\n\n[quality.affected]\n",
        budget.full, budget.affected
    ));
    for (glob, tokens) in default_affected(capabilities) {
        let list = tokens
            .iter()
            .map(|token| format!("\"{token}\""))
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&format!("\"{glob}\" = [{list}]\n"));
    }
    text
}

fn known_tokens(capabilities: &Capabilities, quality: &QualityManifest) -> Vec<&'static str> {
    let mut tokens = vec![
        "fast",
        "scripts-tests",
        "markdown-links",
        "preflight",
        "metric-names",
    ];
    if capabilities.backend {
        tokens.extend([
            "openapi",
            "rust-fmt",
            "migrations",
            "backend",
            "rust-clippy",
            "rust-msrv",
            "rust-tests",
            "rust-doctests",
            "images",
        ]);
        if capabilities.mcp {
            tokens.push("mcp-drift");
        }
    }
    if capabilities.web {
        tokens.extend([
            "spec-purposes",
            "web",
            "web-e2e",
            "playwright",
            "web-install",
            "web-build",
            "web-lint",
            "web-unit",
            "web-service-worker",
            "webkit-repeats",
        ]);
    }
    if capabilities.mobile {
        tokens.extend([
            "mobile",
            "mobile-install",
            "mobile-typecheck",
            "mobile-lint",
            "expo-doctor",
            "mobile-unit",
            "ios-check",
            "android-build",
            "android-e2e",
        ]);
        if !capabilities.web {
            tokens.push("mobile-service-worker");
        }
    }
    if quality.full_stack_e2e {
        tokens.extend(["stack", "full-stack-e2e"]);
    }
    tokens
}

fn invalid_glob(glob: &str) -> bool {
    glob.is_empty()
        || Path::new(glob).is_absolute()
        || glob.split('/').any(|part| part == ".." || part.is_empty())
}

pub fn validate(
    quality: &QualityManifest,
    capabilities: &Capabilities,
    failures: &mut Vec<String>,
) {
    if let Some(port) = quality.android_emulator_port {
        if !capabilities.mobile {
            failures
                .push("quality.android_emulator_port requires the mobile capability".to_owned());
        } else if !(FIRST_EMULATOR_PORT..=LAST_EMULATOR_PORT).contains(&port) || port % 2 != 0 {
            failures.push(format!(
                "quality.android_emulator_port must be an even port from {FIRST_EMULATOR_PORT} to {LAST_EMULATOR_PORT}"
            ));
        }
    }
    if let Some(count) = quality.android_emulators {
        let last_port = u32::from(quality.android_emulator_port.unwrap_or(FIRST_EMULATOR_PORT))
            + 2 * u32::from(count.saturating_sub(1));
        if !capabilities.mobile {
            failures.push("quality.android_emulators requires the mobile capability".to_owned());
        } else if !(1..=MAX_ANDROID_EMULATORS).contains(&count) {
            failures.push(format!(
                "quality.android_emulators must be from 1 to {MAX_ANDROID_EMULATORS}"
            ));
        } else if last_port > u32::from(LAST_EMULATOR_PORT) {
            failures.push(format!(
                "quality.android_emulators needs ports up to {last_port}, past {LAST_EMULATOR_PORT}"
            ));
        }
    }
    let budget = &quality.gate_budget_minutes;
    if budget.full == 0 || budget.affected == 0 {
        failures.push("quality.gate_budget_minutes values must be greater than zero".to_owned());
    }
    let known = known_tokens(capabilities, quality);
    for (glob, tokens) in &quality.affected {
        if invalid_glob(glob) {
            failures.push(format!(
                "quality.affected glob `{glob}` must be a relative path without `..`"
            ));
        }
        for token in tokens {
            let valid = match token.split_once(':') {
                Some(("playwright", tag)) => {
                    capabilities.web && tag.starts_with('@') && tag.len() > 1
                }
                Some(_) => false,
                None => known.contains(&token.as_str()),
            };
            if !valid {
                failures.push(format!(
                    "quality.affected `{glob}` names unknown lane or step `{token}`"
                ));
            }
        }
    }
    for glob in &quality.shared {
        if invalid_glob(glob) {
            failures.push(format!(
                "quality.shared glob `{glob}` must be a relative path without `..`"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities(backend: bool, web: bool, mobile: bool) -> Capabilities {
        toml::from_str(&format!(
            "analytics = \"posthog\"\nbackend = {backend}\nworker = false\nmobile = {mobile}\nweb = {web}\npwa = false\n"
        ))
        .expect("capabilities parse")
    }

    #[test]
    fn emulator_ports_are_even_in_range_and_differ_between_products() {
        let ports: Vec<u16> = [
            "architecture-health-platform",
            "eigenruhe",
            "hebkit",
            "kassensturz",
            "leitbild",
            "lesezeichen",
            "redemut",
            "finops",
            "schlauzug",
            "schluesselbund",
            "solo-leveling-system",
            "tiefgang",
        ]
        .iter()
        .map(|name| default_emulator_port(name))
        .collect();
        for port in &ports {
            assert!((FIRST_EMULATOR_PORT..=LAST_EMULATOR_PORT - 2).contains(port));
            assert_eq!(port % 2, 0);
        }
        let mut unique = ports.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ports.len());
        assert_eq!(
            default_emulator_port("schlauzug"),
            default_emulator_port("schlauzug")
        );
    }

    #[test]
    fn default_table_follows_capabilities() {
        let backend_only = default_affected(&capabilities(true, false, false));
        assert_eq!(backend_only["backend/crates/**"], ["backend"]);
        assert!(!backend_only.contains_key("web/src/**"));
        let all = default_affected(&capabilities(true, true, true));
        assert_eq!(all["mobile/**"], ["mobile"]);
        assert_eq!(all["web/e2e/**"], ["web-e2e"]);
        assert!(all["docs/**"].is_empty());
    }

    #[test]
    fn rendered_defaults_validate() {
        let caps = capabilities(true, true, true);
        let text = format!(
            "profile = \"strict\"\n{}",
            render_quality_extras("demo", &caps, true)
        );
        let quality: QualityManifest = toml::from_str(&text).expect("quality parses");
        assert_eq!(quality.gate_budget_minutes, GateBudgetMinutes::default());
        assert_eq!(
            quality.android_emulator_port,
            Some(default_emulator_port("demo"))
        );
        assert_eq!(quality.android_emulators, Some(2));
        assert_eq!(quality.gate_budget_minutes.full, 10);
        assert_eq!(quality.gate_budget_minutes.affected, 5);
        let mut failures = Vec::new();
        validate(&quality, &caps, &mut failures);
        assert!(failures.is_empty(), "{failures:?}");
    }

    #[test]
    fn invalid_settings_are_reported() {
        let caps = capabilities(true, false, false);
        let quality: QualityManifest = toml::from_str(
            "android_emulator_port = 5557\nshared = [\"../x\"]\n[gate_budget_minutes]\nfull = 0\n[affected]\n\"/abs\" = [\"backend\"]\n\"web/**\" = [\"web\", \"playwright:@x\", \"nope\"]\n",
        )
        .expect("quality parses");
        let mut failures = Vec::new();
        validate(&quality, &caps, &mut failures);
        let text = failures.join("\n");
        assert!(text.contains("android_emulator_port requires the mobile capability"));
        assert!(text.contains("gate_budget_minutes values must be greater than zero"));
        assert!(text.contains("glob `/abs` must be a relative path"));
        assert!(text.contains("unknown lane or step `web`"));
        assert!(text.contains("unknown lane or step `playwright:@x`"));
        assert!(text.contains("unknown lane or step `nope`"));
        assert!(text.contains("quality.shared glob `../x`"));
    }

    #[test]
    fn emulator_counts_must_fit_the_port_range() {
        let caps = capabilities(false, false, true);
        for (text, failures_expected) in [
            ("android_emulators = 2\n", 0),
            ("android_emulators = 0\n", 1),
            ("android_emulators = 5\n", 1),
            ("android_emulator_port = 5682\nandroid_emulators = 2\n", 1),
            ("android_emulator_port = 5676\nandroid_emulators = 4\n", 0),
        ] {
            let quality: QualityManifest = toml::from_str(text).expect("parses");
            let mut failures = Vec::new();
            validate(&quality, &caps, &mut failures);
            assert_eq!(failures.len(), failures_expected, "{text}: {failures:?}");
        }
        let quality: QualityManifest = toml::from_str("android_emulators = 2\n").expect("parses");
        let mut failures = Vec::new();
        validate(&quality, &capabilities(true, false, false), &mut failures);
        assert_eq!(
            failures,
            ["quality.android_emulators requires the mobile capability"]
        );
    }

    #[test]
    fn odd_or_out_of_range_ports_fail() {
        let caps = capabilities(false, false, true);
        for port in [5557_u16, 5554, 5684] {
            let quality: QualityManifest =
                toml::from_str(&format!("android_emulator_port = {port}\n")).expect("parses");
            let mut failures = Vec::new();
            validate(&quality, &caps, &mut failures);
            assert_eq!(failures.len(), 1, "{port}");
        }
    }
}
