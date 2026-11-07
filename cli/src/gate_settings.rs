use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
};

use serde::{Deserialize, Serialize};

use crate::{AuthProvider, Capabilities, QualityManifest};

const FIRST_EMULATOR_PORT: u16 = 5556;
const EMULATOR_BLOCK_PORTS: u16 = 8;
const PRODUCT_EMULATOR_BLOCKS: u32 = 16;
const LAST_EMULATOR_PORT: u16 = 5810;
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

/// First console port of one of the 16 product blocks from 5556 to 5683, picked by name.
/// A block holds 8 ports: the console and ADB ports of up to 4 emulators.
pub fn default_emulator_port(product: &str) -> u16 {
    let hash = product.bytes().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(FNV_PRIME)
    });
    let block = u16::try_from(hash % PRODUCT_EMULATOR_BLOCKS).unwrap_or_default();
    FIRST_EMULATOR_PORT + EMULATOR_BLOCK_PORTS * block
}

/// Console and ADB ports the emulators take, and the block that must hold them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmulatorPorts {
    pub first: u32,
    pub last: u32,
    pub block_first: u32,
    pub block_last: u32,
}

pub fn emulator_ports(port: u16, emulators: u8) -> EmulatorPorts {
    let first = u32::from(port);
    let block = u32::from(EMULATOR_BLOCK_PORTS);
    let block_first = u32::from(FIRST_EMULATOR_PORT)
        + (first.saturating_sub(u32::from(FIRST_EMULATOR_PORT)) / block) * block;
    EmulatorPorts {
        first,
        last: first + 2 * u32::from(emulators.max(1)) - 1,
        block_first,
        block_last: block_first + block - 1,
    }
}

/// Doctor's line for the emulator ports a mobile product's settings take.
pub fn emulator_summary(quality: &QualityManifest, capabilities: &Capabilities) -> Option<String> {
    let port = quality
        .android_emulator_port
        .filter(|_| capabilities.components.mobile())?;
    let emulators = quality
        .android_emulators
        .unwrap_or(DEFAULT_ANDROID_EMULATORS);
    let ports = emulator_ports(port, emulators);
    Some(format!(
        "quality.android_emulator_port {port} with {emulators} emulators takes ports {} to {} in block {} to {}",
        ports.first, ports.last, ports.block_first, ports.block_last
    ))
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
    if capabilities.components.backend() {
        add("backend/crates/**", &["backend"]);
        add("backend/Cargo.toml", &["backend", "images"]);
        add("backend/Dockerfile", &["images"]);
    }
    if capabilities.components.web() {
        add("web/src/**", &["web"]);
        add("web/public/**", &["web"]);
        add("web/index.html", &["web"]);
        add("web/e2e/**", &["web-e2e"]);
    }
    if capabilities.components.mobile() {
        add("mobile/**", &["mobile"]);
        add(
            "mobile/.maestro/**",
            &["mobile", "android-build", "android-e2e"],
        );
    }
    affected
}

pub fn render_quality_extras(name: &str, capabilities: &Capabilities, strict: bool) -> String {
    let mut text = String::new();
    if capabilities.components.mobile() {
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

/// Lanes, steps and selection tokens a product's gate defines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GateSteps {
    pub lanes: BTreeSet<String>,
    pub steps: BTreeSet<String>,
    pub tokens: BTreeSet<String>,
}

const GATE_SCRIPT: &str = "scripts/quality-gate.sh";
const STEP_FIELDS: usize = 5;

/// Parses `name lane after mode tokens` lines from `quality-gate.sh --list-steps`.
pub fn parse_gate_steps(listing: &str) -> Result<GateSteps, String> {
    let mut gate = GateSteps::default();
    for line in listing.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [name, lane, _, _, tokens] = fields[..] else {
            return Err(format!(
                "{GATE_SCRIPT} --list-steps printed `{line}`, expected {STEP_FIELDS} fields"
            ));
        };
        gate.steps.insert(name.to_owned());
        gate.lanes.insert(lane.to_owned());
        gate.tokens.extend([name.to_owned(), lane.to_owned()]);
        if tokens != "-" {
            gate.tokens.extend(
                tokens
                    .split(',')
                    .filter(|token| !token.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    Ok(gate)
}

/// Lists the product gate's steps, or `None` when the product has no strict gate script.
pub fn list_gate_steps(root: &Path) -> Option<Result<GateSteps, String>> {
    if !root.join(GATE_SCRIPT).is_file() {
        return None;
    }
    let output = match Command::new("sh")
        .args([GATE_SCRIPT, "--list-steps"])
        .current_dir(root)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return Some(Err(format!(
                "could not run {GATE_SCRIPT} --list-steps: {error}"
            )));
        }
    };
    if !output.status.success() {
        return Some(Err(format!(
            "{GATE_SCRIPT} --list-steps failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Some(parse_gate_steps(&String::from_utf8_lossy(&output.stdout)))
}

const STEP_TABLE_START: &str = "cat <<'STEPS'\n";
const STEP_TABLE_END: &str = "STEPS\n";

/// Renders the template gate's step table for `capabilities` and returns its step names.
pub fn template_steps(
    gate_template: &str,
    capabilities: &Capabilities,
) -> Result<BTreeSet<String>, String> {
    let table = gate_template
        .split_once(STEP_TABLE_START)
        .and_then(|(_, tail)| tail.split_once(STEP_TABLE_END))
        .map(|(table, _)| table)
        .ok_or("the template gate has no step table")?;
    let context = minijinja::context! { context => minijinja::context! {
        backend => capabilities.components.backend(),
        mcp => capabilities.components.mcp(),
        mobile => capabilities.components.mobile(),
        web => capabilities.components.web(),
        auth_enabled => capabilities.auth.is_some(),
        auth_oidc => capabilities.auth == Some(AuthProvider::Oidc),
    }};
    let rendered = minijinja::Environment::new()
        .render_str(table, context)
        .map_err(|error| format!("could not render the template step table: {error}"))?;
    Ok(parse_gate_steps(&rendered)?.steps)
}

/// Reports template steps that the product's gate does not list.
pub fn validate_template_steps(
    template: &BTreeSet<String>,
    gate: &GateSteps,
    failures: &mut Vec<String>,
) {
    let missing: Vec<&str> = template
        .difference(&gate.steps)
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        failures.push(format!(
            "{GATE_SCRIPT} --list-steps lacks template steps for the declared capabilities: {}",
            missing.join(", ")
        ));
    }
}

fn known_tokens(capabilities: &Capabilities, quality: &QualityManifest) -> Vec<&'static str> {
    let mut tokens = vec![
        "fast",
        "scripts-tests",
        "markdown-links",
        "preflight",
        "baukit-doctor",
        "metric-names",
    ];
    if capabilities.components.backend() {
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
        if capabilities.components.mcp() {
            tokens.push("mcp-drift");
        }
    }
    if capabilities.components.web() {
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
    if capabilities.components.mobile() {
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
        if !capabilities.components.web() {
            tokens.push("mobile-service-worker");
        }
    }
    if matches!(
        capabilities.auth,
        Some(AuthProvider::Clerk | AuthProvider::Workos)
    ) {
        if capabilities.components.web() {
            tokens.push("web-bundle");
        }
        if capabilities.components.mobile() {
            tokens.push("mobile-bundle");
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
    gate: Option<&GateSteps>,
    failures: &mut Vec<String>,
) {
    if let Some(port) = quality.android_emulator_port {
        if !capabilities.components.mobile() {
            failures
                .push("quality.android_emulator_port requires the mobile capability".to_owned());
        } else if !(FIRST_EMULATOR_PORT..=LAST_EMULATOR_PORT).contains(&port) || port % 2 != 0 {
            failures.push(format!(
                "quality.android_emulator_port must be an even port from {FIRST_EMULATOR_PORT} to {LAST_EMULATOR_PORT}"
            ));
        }
    }
    if quality.android_wifi && !capabilities.components.mobile() {
        failures.push("quality.android_wifi requires the mobile capability".to_owned());
    }
    let count = quality
        .android_emulators
        .unwrap_or(DEFAULT_ANDROID_EMULATORS);
    if quality.android_emulators.is_some() && !capabilities.components.mobile() {
        failures.push("quality.android_emulators requires the mobile capability".to_owned());
    } else if !(1..=MAX_ANDROID_EMULATORS).contains(&count) {
        failures.push(format!(
            "quality.android_emulators must be from 1 to {MAX_ANDROID_EMULATORS}"
        ));
    } else if capabilities.components.mobile() {
        let ports = emulator_ports(
            quality.android_emulator_port.unwrap_or(FIRST_EMULATOR_PORT),
            count,
        );
        if ports.last > ports.block_last {
            failures.push(format!(
                "quality.android_emulator_port {} with {count} emulators takes ports {} to {}, past its block {} to {}; start the range at {}",
                ports.first, ports.first, ports.last, ports.block_first, ports.block_last, ports.block_first
            ));
        }
    }
    let budget = &quality.gate_budget_minutes;
    if budget.full == 0 || budget.affected == 0 {
        failures.push("quality.gate_budget_minutes values must be greater than zero".to_owned());
    }
    let known: BTreeSet<String> = gate.map_or_else(
        || {
            known_tokens(capabilities, quality)
                .into_iter()
                .map(str::to_owned)
                .collect()
        },
        |gate| gate.tokens.clone(),
    );
    for (glob, tokens) in &quality.affected {
        if invalid_glob(glob) {
            failures.push(format!(
                "quality.affected glob `{glob}` must be a relative path without `..`"
            ));
        }
        for token in tokens {
            let valid = match token.split_once(':') {
                Some(("playwright", tag)) => {
                    known.contains("playwright") && tag.starts_with('@') && tag.len() > 1
                }
                Some(_) => false,
                None => known.contains(token),
            };
            if !valid {
                failures.push(format!(
                    "quality.affected `{glob}` names unknown lane or step `{token}`"
                ));
            }
        }
    }
    let lanes_and_steps: BTreeSet<String> = gate.map_or_else(
        || known.clone(),
        |gate| gate.lanes.union(&gate.steps).cloned().collect(),
    );
    for name in &quality.light {
        if !lanes_and_steps.contains(name) {
            failures.push(format!("quality.light names unknown lane or step `{name}`"));
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
    fn default_emulator_ports_start_a_product_block() {
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
        let blocks: Vec<u16> = ports
            .iter()
            .map(|port| (port - FIRST_EMULATOR_PORT) / EMULATOR_BLOCK_PORTS)
            .collect();
        for (port, block) in ports.iter().zip(&blocks) {
            assert_eq!(
                (port - FIRST_EMULATOR_PORT) % EMULATOR_BLOCK_PORTS,
                0,
                "{port}"
            );
            assert!(u32::from(*block) < PRODUCT_EMULATOR_BLOCKS, "{port}");
            let range = emulator_ports(*port, MAX_ANDROID_EMULATORS);
            assert_eq!(
                (range.first, range.last),
                (range.block_first, range.block_last)
            );
        }
        let mut unique = blocks.clone();
        unique.sort_unstable();
        unique.dedup();
        assert!(unique.len() > 1);
        assert_eq!(
            default_emulator_port("schlauzug"),
            default_emulator_port("schlauzug")
        );
    }

    #[test]
    fn emulator_ranges_must_stay_in_their_block() {
        let caps = capabilities(false, false, true);
        for (text, expected) in [
            ("android_emulator_port = 5644\n", None),
            (
                "android_emulator_port = 5646\nandroid_emulators = 3\n",
                None,
            ),
            ("android_emulator_port = 5790\n", None),
            (
                "android_emulator_port = 5594\n",
                Some(
                    "quality.android_emulator_port 5594 with 2 emulators takes ports 5594 to 5597, past its block 5588 to 5595; start the range at 5588",
                ),
            ),
            (
                "android_emulator_port = 5644\nandroid_emulators = 4\n",
                None,
            ),
            (
                "android_emulator_port = 5646\nandroid_emulators = 4\n",
                Some(
                    "quality.android_emulator_port 5646 with 4 emulators takes ports 5646 to 5653, past its block 5644 to 5651; start the range at 5644",
                ),
            ),
        ] {
            let quality: QualityManifest = toml::from_str(text).expect("parses");
            let mut failures = Vec::new();
            validate(&quality, &caps, None, &mut failures);
            assert_eq!(failures, expected.into_iter().collect::<Vec<_>>(), "{text}");
        }
        let quality: QualityManifest =
            toml::from_str("android_emulator_port = 5790\n").expect("parses");
        assert_eq!(
            emulator_summary(&quality, &caps).as_deref(),
            Some(
                "quality.android_emulator_port 5790 with 2 emulators takes ports 5790 to 5793 in block 5788 to 5795"
            )
        );
        assert_eq!(
            emulator_summary(&quality, &capabilities(true, false, false)),
            None
        );
    }

    #[test]
    fn default_table_follows_capabilities() {
        let backend_only = default_affected(&capabilities(true, false, false));
        assert_eq!(backend_only["backend/crates/**"], ["backend"]);
        assert!(!backend_only.contains_key("web/src/**"));
        let all = default_affected(&capabilities(true, true, true));
        assert_eq!(all["mobile/**"], ["mobile"]);
        assert_eq!(
            all["mobile/.maestro/**"],
            ["mobile", "android-build", "android-e2e"]
        );
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
        validate(&quality, &caps, None, &mut failures);
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
        validate(&quality, &caps, None, &mut failures);
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
            ("android_emulator_port = 5810\nandroid_emulators = 2\n", 1),
            ("android_emulator_port = 5804\nandroid_emulators = 4\n", 0),
        ] {
            let quality: QualityManifest = toml::from_str(text).expect("parses");
            let mut failures = Vec::new();
            validate(&quality, &caps, None, &mut failures);
            assert_eq!(failures.len(), failures_expected, "{text}: {failures:?}");
        }
        let quality: QualityManifest = toml::from_str("android_emulators = 2\n").expect("parses");
        let mut failures = Vec::new();
        validate(
            &quality,
            &capabilities(true, false, false),
            None,
            &mut failures,
        );
        assert_eq!(
            failures,
            ["quality.android_emulators requires the mobile capability"]
        );
    }

    #[test]
    fn android_wifi_requires_the_mobile_capability() {
        let quality: QualityManifest = toml::from_str("android_wifi = true\n").expect("parses");
        let mut failures = Vec::new();
        validate(
            &quality,
            &capabilities(false, false, true),
            None,
            &mut failures,
        );
        assert!(failures.is_empty(), "{failures:?}");
        validate(
            &quality,
            &capabilities(true, false, false),
            None,
            &mut failures,
        );
        assert_eq!(
            failures,
            ["quality.android_wifi requires the mobile capability"]
        );
    }

    #[test]
    fn odd_or_out_of_range_ports_fail() {
        let caps = capabilities(false, false, true);
        for port in [5557_u16, 5554, 5812] {
            let quality: QualityManifest =
                toml::from_str(&format!("android_emulator_port = {port}\n")).expect("parses");
            let mut failures = Vec::new();
            validate(&quality, &caps, None, &mut failures);
            assert_eq!(failures.len(), 1, "{port}");
        }
    }

    #[test]
    fn product_steps_from_the_gate_listing_are_accepted() {
        let gate = parse_gate_steps(
            "openapi fast - always -\nwebkit-repeats web playwright selected -\nmobile-cpu-budgets mobile - selected cpu\n\nplaywright web - selected web-e2e\n",
        )
        .expect("listing parses");
        assert_eq!(
            gate.lanes,
            BTreeSet::from(["fast".into(), "mobile".into(), "web".into()])
        );
        assert!(gate.steps.contains("mobile-cpu-budgets"));
        assert!(gate.tokens.contains("cpu") && gate.tokens.contains("web-e2e"));
        let quality: QualityManifest = toml::from_str(
            "[affected]\n\"mobile/**\" = [\"mobile-cpu-budgets\", \"cpu\", \"playwright:@smoke\"]\n\"web/**\" = [\"rust-tests\", \"unknown:x\"]\n",
        )
        .expect("quality parses");
        let mut failures = Vec::new();
        validate(
            &quality,
            &capabilities(false, false, true),
            Some(&gate),
            &mut failures,
        );
        assert_eq!(
            failures,
            [
                "quality.affected `web/**` names unknown lane or step `rust-tests`",
                "quality.affected `web/**` names unknown lane or step `unknown:x`",
            ]
        );
    }

    #[test]
    fn light_names_must_be_lanes_or_steps_of_the_gate() {
        let gate = parse_gate_steps("doctor fast - always -\nplaywright web - selected web-e2e\n")
            .expect("listing parses");
        let quality: QualityManifest =
            toml::from_str("light = [\"web\", \"doctor\", \"web-e2e\", \"nope\"]\n")
                .expect("quality parses");
        let mut failures = Vec::new();
        validate(
            &quality,
            &capabilities(false, true, false),
            Some(&gate),
            &mut failures,
        );
        assert_eq!(
            failures,
            [
                "quality.light names unknown lane or step `web-e2e`",
                "quality.light names unknown lane or step `nope`",
            ]
        );
        let quality: QualityManifest =
            toml::from_str("light = [\"web\", \"nope\"]\n").expect("quality parses");
        let mut failures = Vec::new();
        validate(
            &quality,
            &capabilities(false, true, false),
            None,
            &mut failures,
        );
        assert_eq!(
            failures,
            ["quality.light names unknown lane or step `nope`"]
        );
    }

    #[test]
    fn malformed_gate_listings_are_rejected() {
        let error = parse_gate_steps("openapi fast always -\n").expect_err("four fields");
        assert!(error.contains("expected 5 fields"), "{error}");
    }

    #[test]
    fn gate_steps_come_from_the_product_script() {
        let directory = tempfile::tempdir().expect("temporary directory");
        assert!(list_gate_steps(directory.path()).is_none());
        let scripts = directory.path().join("scripts");
        std::fs::create_dir_all(&scripts).expect("scripts directory");
        std::fs::write(
            scripts.join("quality-gate.sh"),
            "[ \"$1\" = --list-steps ] || exit 3\necho 'doctor fast - always -'\n",
        )
        .expect("script");
        let gate = list_gate_steps(directory.path())
            .expect("script exists")
            .expect("listing runs");
        assert!(gate.steps.contains("doctor"));
        std::fs::write(scripts.join("quality-gate.sh"), "echo broken >&2\nexit 4\n")
            .expect("script");
        let error = list_gate_steps(directory.path())
            .expect("script exists")
            .expect_err("listing fails");
        assert!(error.contains("--list-steps failed: broken"), "{error}");
    }

    #[test]
    fn template_steps_follow_capabilities() {
        let template = include_str!("../../templates/common/__strict__/scripts/quality-gate.sh");
        let mut clerk = capabilities(false, false, true);
        clerk.auth = Some(AuthProvider::Clerk);
        let steps = template_steps(template, &clerk).expect("renders");
        assert!(steps.contains("expo-doctor") && steps.contains("mobile-bundle"));
        assert!(!steps.contains("rust-clippy") && !steps.contains("web-build"));
        clerk.auth = Some(AuthProvider::Oidc);
        let steps = template_steps(template, &clerk).expect("renders");
        assert!(!steps.contains("mobile-bundle"));
        assert!(template_steps("no table", &clerk).is_err());
    }

    #[test]
    fn missing_template_steps_are_reported() {
        let template = BTreeSet::from(["expo-doctor".to_owned(), "doctor".to_owned()]);
        let gate = parse_gate_steps("doctor fast - always -\n").expect("parses");
        let mut failures = Vec::new();
        validate_template_steps(&template, &gate, &mut failures);
        assert_eq!(
            failures,
            [format!(
                "{GATE_SCRIPT} --list-steps lacks template steps for the declared capabilities: expo-doctor"
            )]
        );
    }
}
